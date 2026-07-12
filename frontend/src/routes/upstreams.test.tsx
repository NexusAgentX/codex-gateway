import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { expect, it } from "vitest";
import { settings } from "../test/fixtures";
import { renderWithSession } from "../test/render";
import { server } from "../test/server";
import { UpstreamsPage } from "./upstreams";

it("creates an upstream using the runtime timeout mode", async () => {
  let createBody: Record<string, unknown> | undefined;
  server.use(
    http.get("/api/admin/upstreams", () => HttpResponse.json([])),
    http.get("/api/admin/settings", () => HttpResponse.json(settings)),
    http.post("/api/admin/upstreams", async ({ request }) => {
      createBody = await request.json() as Record<string, unknown>;
      return HttpResponse.json({});
    })
  );
  const user = userEvent.setup();
  renderWithSession(<UpstreamsPage />);

  await screen.findByText("No upstreams configured.");
  await user.type(screen.getByPlaceholderText("Name"), "Primary");
  await user.type(screen.getByPlaceholderText("Base URL"), "https://upstream.example");
  await user.type(screen.getByPlaceholderText("API key"), "sk-upstream");
  await user.click(screen.getByRole("button", { name: "Create" }));
  expect(createBody).toMatchObject({ name: "Primary", base_url: "https://upstream.example", api_key: "sk-upstream" });
  expect(createBody).not.toHaveProperty("timeout_ms");
});
