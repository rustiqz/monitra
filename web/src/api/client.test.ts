import { afterEach, describe, expect, it, vi } from "vitest";
import { clearToken, getToken, setToken, verifyToken } from "./client";

function stubStorage(impl: Partial<Storage>) {
  vi.stubGlobal("window", { localStorage: impl });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("token storage", () => {
  it("round-trips a token through localStorage", () => {
    const data = new Map<string, string>();
    stubStorage({
      getItem: (k) => data.get(k) ?? null,
      setItem: (k, v) => void data.set(k, v),
      removeItem: (k) => void data.delete(k),
    });
    expect(getToken()).toBeNull();
    setToken("abc");
    expect(getToken()).toBe("abc");
    clearToken();
    expect(getToken()).toBeNull();
  });

  // The client's documented contract: a blocked or throwing store degrades to
  // "ask for the token again", never a crash.
  it("treats a throwing store as no token and does not throw", () => {
    const boom = () => {
      throw new Error("blocked");
    };
    stubStorage({ getItem: boom, setItem: boom, removeItem: boom });
    expect(getToken()).toBeNull();
    expect(() => setToken("abc")).not.toThrow();
    expect(() => clearToken()).not.toThrow();
  });
});

describe("verifyToken", () => {
  it("sends the token as a bearer header and reports acceptance", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true });
    vi.stubGlobal("fetch", fetchMock);
    expect(await verifyToken("secret")).toBe(true);
    expect(fetchMock).toHaveBeenCalledWith("/monitors", {
      headers: { Authorization: "Bearer secret" },
    });
  });

  it("reports rejection when the API says no", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false }));
    expect(await verifyToken("wrong")).toBe(false);
  });
});
