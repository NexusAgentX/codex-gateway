import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../test/server";
import { SessionProvider, useSession } from "./session";

function SessionHarness() {
  const { session, checkingSession, login, logout } = useSession();
  return (
    <div>
      <span>{checkingSession ? "checking" : session?.user.email ?? "signed-out"}</span>
      <button type="button" onClick={() => login({ token: "new-token", token_type: "panel", user: { id: "u1", email: "new@example.com", role: "admin" } })}>Login</button>
      <button type="button" onClick={logout}>Logout</button>
    </div>
  );
}

describe("SessionProvider", () => {
  it("persists login and clears logout", async () => {
    const user = userEvent.setup();
    render(<SessionProvider><SessionHarness /></SessionProvider>);
    await user.click(screen.getByRole("button", { name: "Login" }));
    expect(screen.getByText("new@example.com")).toBeInTheDocument();
    expect(localStorage.getItem("codex-gateway-session")).toContain("new-token");
    await user.click(screen.getByRole("button", { name: "Logout" }));
    expect(screen.getByText("signed-out")).toBeInTheDocument();
    expect(localStorage.getItem("codex-gateway-session")).toBeNull();
  });

  it("refreshes a stored session from /api/me", async () => {
    localStorage.setItem("codex-gateway-session", JSON.stringify({ token: "stored", user: { id: "old", email: "old@example.com", role: "user" } }));
    server.use(http.get("/api/me", () => HttpResponse.json({ user_id: "u2", email: "fresh@example.com", role: "admin" })));
    render(<SessionProvider><SessionHarness /></SessionProvider>);
    expect(screen.getByText("checking")).toBeInTheDocument();
    expect(await screen.findByText("fresh@example.com")).toBeInTheDocument();
    expect(localStorage.getItem("codex-gateway-session")).toContain("fresh@example.com");
  });

  it("drops a stored session after a 401", async () => {
    localStorage.setItem("codex-gateway-session", JSON.stringify({ token: "expired", user: { id: "u1", email: "old@example.com", role: "user" } }));
    server.use(http.get("/api/me", () => HttpResponse.json({ error: { code: "invalid_api_key" } }, { status: 401 })));
    render(<SessionProvider><SessionHarness /></SessionProvider>);
    await waitFor(() => expect(screen.getByText("signed-out")).toBeInTheDocument());
    expect(localStorage.getItem("codex-gateway-session")).toBeNull();
  });
});
