import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import type { ReactElement } from "react";
import { SessionProvider, type Session } from "../lib/auth/session";
import { server } from "./server";

export const adminSession: Session = {
  token: "panel-admin",
  user: { id: "user-1", email: "admin@example.com", role: "admin" }
};

export const userSession: Session = {
  token: "panel-user",
  user: { id: "user-2", email: "user@example.com", role: "user" }
};

export function storeSession(session: Session) {
  localStorage.setItem("codex-gateway-session", JSON.stringify(session));
  server.use(
    http.get("/api/me", () =>
      HttpResponse.json({
        user_id: session.user.id,
        email: session.user.email,
        role: session.user.role
      })
    )
  );
}

export function renderWithSession(ui: ReactElement, session: Session = adminSession) {
  storeSession(session);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } }
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <SessionProvider>{ui}</SessionProvider>
    </QueryClientProvider>
  );
}
