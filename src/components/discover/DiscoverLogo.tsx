import { useState } from "react";
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
  const [failedSrc, setFailedSrc] = useState<string | null>(null);
  const usable = src && src.startsWith("https://") && failedSrc !== src;
  const radius = size === 44 ? 11 : size === 36 ? 9 : 7;
  const isSvg = Boolean(src && /\.svg(?:$|[?#])/i.test(src));
  return (
    <span
      aria-hidden="true"
      style={{ width: size, height: size, borderRadius: radius }}
      className={cn(
        "relative inline-flex shrink-0 select-none items-center justify-center overflow-hidden",
        "after:pointer-events-none after:absolute after:inset-0 after:rounded-[inherit] after:shadow-[inset_0_0_0_1px_rgba(0,0,0,0.07)] dark:after:shadow-[inset_0_0_0_1px_rgba(255,255,255,0.08)]",
        usable ? "bg-white" : "bg-subtle text-fg-2",
        className,
      )}
    >
      {usable ? (
        <img
          src={src}
          alt=""
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          draggable={false}
          onError={() => setFailedSrc(src)}
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
