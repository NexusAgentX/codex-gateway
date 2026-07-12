import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router-dom";
import { expect, it } from "vitest";
import { SessionProvider } from "../lib/auth/session";
import { storeSession, userSession } from "../test/render";
import { appRoutes } from "./router";

function renderRoute(path: string) {
  const router = createMemoryRouter(appRoutes, { initialEntries: [path] });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <SessionProvider><RouterProvider router={router} /></SessionProvider>
    </QueryClientProvider>
  );
}

it("redirects signed-out users and blocks non-admin settings access", async () => {
  renderRoute("/settings");
  expect(await screen.findByRole("button", { name: "Sign in" })).toBeInTheDocument();

  storeSession(userSession);
  renderRoute("/settings");
  expect(await screen.findByRole("heading", { name: "Admin only" })).toBeInTheDocument();
  expect(screen.getByText("This page requires an admin account.")).toBeInTheDocument();
});
