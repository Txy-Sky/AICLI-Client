import { useCallback, useEffect, useRef, useState } from "react";
import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { aicliApi, AICLI_EVENTS } from "@/lib/api/aicli";
import { TOPUP_ORDER_GRACE_MS, TOPUP_ORDER_VALID_MS } from "@/lib/aicliWallet";
import type {
  AicliPayFinished,
  AicliPayKind,
  AicliPayStarted,
} from "@/types/aicli";
import { aicliKeys } from "./aicliKeys";
import { useTauriEvent } from "./useTauriEvent";

/** 订单轮询间隔。 */
export const PAY_POLL_MS = 3_000;
/** 易支付订单 5 分钟不付款即超时：到点关闭支付窗口。 */
export const PAY_OPEN_LIMIT_MS = TOPUP_ORDER_VALID_MS;
/** 支付窗口结束后再确认这么久（等服务端收到异步通知）。 */
export const PAY_AFTER_CLOSE_MS = TOPUP_ORDER_GRACE_MS;
/** 超时检查的节拍；每 PAY_POLL_MS 才真正查一次订单。 */
const PAY_TICK_MS = 1_000;

/** 充值设置、订阅套餐、我的订阅、邀请码（实时读取，不落本地缓存）。 */
export function useAicliWallet(enabled = true) {
  return useQuery({
    queryKey: aicliKeys.wallet,
    queryFn: () => aicliApi.walletGet(),
    enabled,
    placeholderData: (previous) => previous,
  });
}

export function useAicliWalletHistory(page: number, enabled = true) {
  return useQuery({
    queryKey: aicliKeys.walletHistory(page),
    queryFn: () => aicliApi.walletHistory(page),
    enabled,
    placeholderData: keepPreviousData,
  });
}

/** 实付金额；amount 为空或不是正整数时不查询。 */
export function useAicliWalletQuote(amount: number | null) {
  const valid = amount != null && Number.isInteger(amount) && amount > 0;
  return useQuery({
    queryKey: aicliKeys.walletQuote(valid ? amount : 0),
    queryFn: () => aicliApi.walletQuote(amount as number),
    enabled: valid,
    retry: false,
    staleTime: 30_000,
  });
}

function useInvalidateWallet() {
  const queryClient = useQueryClient();
  return useCallback(
    () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: aicliKeys.wallet }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.walletHistoryAll }),
        queryClient.invalidateQueries({ queryKey: aicliKeys.account }),
        queryClient.invalidateQueries({
          queryKey: aicliKeys.cache("account"),
        }),
      ]),
    [queryClient],
  );
}

export function useAicliWalletPay() {
  return useMutation({
    mutationFn: ({ amount, method }: { amount: number; method: string }) =>
      aicliApi.walletPay(amount, method),
  });
}

export function useAicliWalletRedeem() {
  const invalidate = useInvalidateWallet();
  return useMutation({
    mutationFn: (key: string) => aicliApi.walletRedeem(key),
    onSuccess: () => invalidate(),
  });
}

export function useAicliSubscriptionBuy() {
  const invalidate = useInvalidateWallet();
  return useMutation({
    mutationFn: ({ planId, method }: { planId: number; method: string }) =>
      aicliApi.subscriptionBuy(planId, method),
    onSuccess: (started) => (started.completed ? invalidate() : undefined),
  });
}

export function useAicliAffTransfer() {
  const invalidate = useInvalidateWallet();
  return useMutation({
    mutationFn: (quota: number) => aicliApi.affTransfer(quota),
    onSuccess: () => invalidate(),
  });
}

export type AicliPayWatch =
  | { phase: "idle" }
  | {
      phase: "waiting";
      kind: AicliPayKind;
      host: string | null;
      /** 支付窗口是否还开着；关掉后进入「确认到账」阶段。 */
      windowOpen: boolean;
      /** 订单超时的时刻（毫秒时间戳）；到点自动关闭支付窗口。 */
      expiresAt: number;
    }
  | { phase: "done"; kind: AicliPayKind; result: "success" | "notDetected" };

interface PaySession {
  kind: AicliPayKind;
  tradeNo: string | null;
  /** 下单前已有的订阅数；多出来即视为购买成功。 */
  subscriptionBaseline: number;
  windowOpen: boolean;
  /** 订单超时的时刻：到点关闭支付窗口。 */
  expiresAt: number;
  /** 窗口结束后确认到账的截止时刻；窗口还开着时为空。 */
  confirmUntil: number | null;
  closing: boolean;
  busy: boolean;
}

/**
 * 等待一笔支付到账：到账以服务端为准（易支付异步通知 new-api），客户端只轮询确认。
 * 充值查这笔订单的状态；订阅没有面向用户的订单接口，看订阅数是否增加。
 * 订单 5 分钟超时：到点关闭支付窗口。支付窗口结束（aicli-pay-finished）后再确认一小段
 * 时间，仍未到账就停下并提示去记录里看。
 */
export function useAicliPayWatch() {
  const invalidate = useInvalidateWallet();
  const [watch, setWatch] = useState<AicliPayWatch>({ phase: "idle" });
  const session = useRef<PaySession | null>(null);

  const finish = useCallback(
    (current: PaySession, result: "success" | "notDetected") => {
      if (session.current !== current) return;
      session.current = null;
      setWatch({ phase: "done", kind: current.kind, result });
      void invalidate();
    },
    [invalidate],
  );

  const check = useCallback(async () => {
    const current = session.current;
    if (!current || current.busy) return;
    current.busy = true;
    let paid = false;
    try {
      if (current.kind === "topup") {
        paid =
          current.tradeNo != null &&
          (await aicliApi.walletOrderStatus(current.tradeNo)) === "success";
      } else {
        const view = await aicliApi.walletGet();
        paid =
          !view.failed.includes("subscriptions") &&
          view.subscriptions.length > current.subscriptionBaseline;
      }
    } catch {
      // 一次查询失败不结束等待；到截止时间再停。
    } finally {
      current.busy = false;
    }
    if (paid) finish(current, "success");
    else if (current.confirmUntil != null && Date.now() >= current.confirmUntil)
      finish(current, "notDetected");
  }, [finish]);

  /** 支付窗口已结束（回跳、被关闭或超时关闭）：进入确认到账阶段。 */
  const markClosed = useCallback(
    (current: PaySession) => {
      if (session.current !== current || !current.windowOpen) return;
      current.windowOpen = false;
      current.confirmUntil = Date.now() + PAY_AFTER_CLOSE_MS;
      setWatch((previous) =>
        previous.phase === "waiting"
          ? { ...previous, windowOpen: false }
          : previous,
      );
      void check();
    },
    [check],
  );

  const waiting = watch.phase === "waiting";
  useEffect(() => {
    if (!waiting) return;
    let ticks = 0;
    const timer = window.setInterval(() => {
      const current = session.current;
      if (!current) return;
      ticks += 1;
      if (
        current.windowOpen &&
        !current.closing &&
        Date.now() >= current.expiresAt
      ) {
        // 订单超时：关掉支付窗口。窗口的关闭事件会再走一遍 markClosed（幂等）。
        current.closing = true;
        void aicliApi
          .walletClosePay()
          .catch(() => false)
          .finally(() => markClosed(current));
        return;
      }
      if (ticks % Math.round(PAY_POLL_MS / PAY_TICK_MS) === 0) void check();
    }, PAY_TICK_MS);
    return () => window.clearInterval(timer);
  }, [waiting, check, markClosed]);

  useTauriEvent<AicliPayFinished>(AICLI_EVENTS.payFinished, () => {
    const current = session.current;
    if (!current) return;
    if (current.windowOpen) markClosed(current);
    else void check();
  });

  const start = useCallback(
    (
      kind: AicliPayKind,
      started: AicliPayStarted,
      subscriptionBaseline = 0,
    ) => {
      const expiresAt = Date.now() + PAY_OPEN_LIMIT_MS;
      session.current = {
        kind,
        tradeNo: started.trade_no,
        subscriptionBaseline,
        windowOpen: true,
        expiresAt,
        confirmUntil: null,
        closing: false,
        busy: false,
      };
      setWatch({
        phase: "waiting",
        kind,
        host: started.host,
        windowOpen: true,
        expiresAt,
      });
    },
    [],
  );

  /** 不再等待：窗口还开着就一并关掉（不留一个没人管的收银台）。 */
  const dismiss = useCallback(() => {
    const current = session.current;
    session.current = null;
    setWatch({ phase: "idle" });
    if (current?.windowOpen) void aicliApi.walletClosePay().catch(() => false);
  }, []);

  return { watch, start, dismiss };
}
