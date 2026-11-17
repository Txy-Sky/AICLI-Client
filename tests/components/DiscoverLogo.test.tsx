import { fireEvent, render, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DiscoverLogo } from "@/components/discover/DiscoverLogo";
import { resetIconCacheMemo } from "@/lib/discoverIconCache";
import { server } from "../msw/server";

const TAURI = "http://tauri.local";
const URL = "https://github.com/acme.png?size=96";

describe("DiscoverLogo", () => {
  beforeEach(() => {
    resetIconCacheMemo();
  });

  it("uses the local cache when the icon was stored before", async () => {
    server.use(
      http.post(`${TAURI}/get_cached_discover_icon`, () =>
        HttpResponse.json("data:image/png;base64,AAAA"),
      ),
    );
    const { container } = render(<DiscoverLogo src={URL} name="Acme" />);
    await waitFor(() =>
      expect(container.querySelector("img")?.getAttribute("src")).toBe(
        "data:image/png;base64,AAAA",
      ),
    );
  });

  it("loads from the network when not cached and stores it after it loads", async () => {
    const stored = vi.fn();
    server.use(
      http.post(`${TAURI}/get_cached_discover_icon`, () =>
        HttpResponse.json(null),
      ),
      http.post(`${TAURI}/cache_discover_icon`, async ({ request }) => {
        stored(await request.json());
        return HttpResponse.json(true);
      }),
    );
    const { container } = render(<DiscoverLogo src={URL} name="Acme" />);
    await waitFor(() =>
      expect(container.querySelector("img")?.getAttribute("src")).toBe(URL),
    );
    expect(stored).not.toHaveBeenCalled();
    fireEvent.load(container.querySelector("img")!);
    await waitFor(() => expect(stored).toHaveBeenCalledWith({ url: URL }));
  });

  it("falls back to the initial when the image fails or there is no https icon", async () => {
    server.use(
      http.post(`${TAURI}/get_cached_discover_icon`, () =>
        HttpResponse.json(null),
      ),
    );
    const { container, rerender } = render(
      <DiscoverLogo src={URL} name="acme" />,
    );
    await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe("A");

    rerender(<DiscoverLogo src="http://insecure.dev/a.png" name="@beta" />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toBe("B");
  });
});
