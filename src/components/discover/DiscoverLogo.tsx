import { useEffect, useState } from "react";
import { getCachedIcon, storeIcon } from "@/lib/discoverIconCache";
import { cn } from "@/lib/utils";

/** GitHub 用户 / 组织头像（96px），卡片图标的默认来源（同 magpie 的 gh(owner)） */
export { githubAvatarUrl as githubAvatar } from "@/lib/mcpRegistry";

/** 首字母：去掉开头的符号，取第一个字母或数字 */
export function logoInitial(name: string): string {
  const trimmed = name.replace(/^[^\p{L}\p{N}]+/u, "");
  return (Array.from(trimmed)[0] ?? "?").toUpperCase();
}

/**
 * 卡片左上角的图标：白底圆角方块 + 一圈很淡的内描边（深浅色各一套）。
 * 先读本地缓存；没有缓存时直接从图标地址加载，加载成功后存进本地缓存。
 * 图片加载失败（或没有图片）时显示首字母方块。只加载 https 图片。
 */
export function DiscoverLogo({
  src,
  name,
  size = 36,
  className,
}: {
  src?: string;
  name: string;
  size?: 26 | 36 | 44;
  className?: string;
}) {
  const remote = src && src.startsWith("https://") ? src : undefined;
  // 先查本地缓存：lookup 为 undefined = 还在查，null = 没有缓存（直接从网络加载）
  const [cached, setCached] = useState<{
    src: string;
    dataUrl: string | null;
  } | null>(null);
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!remote) return;
    let alive = true;
    void getCachedIcon(remote).then((dataUrl) => {
      if (alive) setCached({ src: remote, dataUrl });
    });
    return () => {
      alive = false;
    };
  }, [remote]);
  const lookup = remote && cached?.src === remote ? cached.dataUrl : undefined;
  const checking = Boolean(remote) && lookup === undefined;
  const imageSrc = lookup ?? remote;
  const usable = !checking && imageSrc && failedSrc !== imageSrc;
  const radius = size === 44 ? 11 : size === 36 ? 9 : 7;
  const isSvg = Boolean(src && /\.svg(?:$|[?#])/i.test(src));
  return (
    <span
      aria-hidden="true"
      style={{ width: size, height: size, borderRadius: radius }}
      className={cn(
        "relative inline-flex shrink-0 select-none items-center justify-center overflow-hidden",
        "after:pointer-events-none after:absolute after:inset-0 after:rounded-[inherit] after:shadow-[inset_0_0_0_1px_rgba(0,0,0,0.07)] dark:after:shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)]",
        usable || checking ? "bg-white" : "bg-subtle text-fg-2",
        className,
      )}
    >
      {checking ? null : usable ? (
        <img
          src={imageSrc}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          draggable={false}
          onLoad={() => {
            // 从网络加载成功：存一份到本地，下次不再请求
            if (!lookup && remote) storeIcon(remote);
          }}
          onError={() => setFailedSrc(imageSrc ?? null)}
          className={cn(
            "object-contain",
            isSvg ? "h-[72%] w-[72%]" : "h-full w-full",
          )}
        />
      ) : (
        <span
          className="font-semibold"
          style={{ fontSize: Math.round(size * 0.42) }}
        >
          {logoInitial(name)}
        </span>
      )}
    </span>
  );
}
