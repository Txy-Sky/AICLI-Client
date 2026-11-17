import { useState } from "react";

/**
 * 记在本机 localStorage 的界面选择（页签、视图、时间范围等）：只是界面偏好，不进
 * settings.json、不随云同步。读不到、写不了（隐私模式）或值认不出时按默认值。
 */
export function readRememberedChoice<T extends string>(
  key: string,
  allowed: readonly T[],
  fallback: T,
): T {
  try {
    const stored = window.localStorage.getItem(key);
    return allowed.includes(stored as T) ? (stored as T) : fallback;
  } catch {
    return fallback;
  }
}

export function writeRememberedChoice(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // 写不了就只在本次生效
  }
}

/** 像 useState 一样用，选了就记下；下次打开从记下的值开始 */
export function useRememberedChoice<T extends string>(
  key: string,
  allowed: readonly T[],
  fallback: T,
) {
  const [value, setValue] = useState<T>(() =>
    readRememberedChoice(key, allowed, fallback),
  );
  const update = (next: T) => {
    setValue(next);
    writeRememberedChoice(key, next);
  };
  return [value, update] as const;
}
