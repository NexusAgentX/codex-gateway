import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { expect, it } from "vitest";
import { adminLimits } from "../test/fixtures";
import { renderWithSession } from "../test/render";
import { server } from "../test/server";
import { ApiKeysPage } from "./api-keys";

it("creates an admin API key and reveals its plaintext once", async () => {
  let createBody: unknown;
  server.use(
    http.get("/api/admin/api-keys", () => HttpResponse.json([])),
    http.get("/api/admin/users", () => HttpResponse.json([{ id: "user-1", email: "admin@example.com", role: "admin", status: "active", display_name: null, created_at: "2026-07-12T00:00:00Z", updated_at: "2026-07-12T00:00:00Z", last_login_at: null }])),
    http.get("/api/admin/limits", () => HttpResponse.json(adminLimits)),
    http.post("/api/admin/api-keys", async ({ request }) => {
      createBody = await request.json();
      return HttpResponse.json({ key: { id: "key-1", user_id: "user-1", name: "automation", key_prefix: "abc", status: "active", last_used_at: null, expires_at: null, created_at: "2026-07-12T00:00:00Z", revoked_at: null }, plaintext: "cgk_live_abc_secret" });
    })
  );
  const user = userEvent.setup();
  renderWithSession(<ApiKeysPage />);

  await screen.findByText("No API keys have been created.");
  await user.type(screen.getByPlaceholderText("Key name"), "automation");
  await user.click(screen.getByRole("button", { name: "Create" }));
  expect(await screen.findByText("cgk_live_abc_secret")).toBeInTheDocument();
  expect(createBody).toEqual({ user_id: "user-1", name: "automation", expires_at: null });
});
