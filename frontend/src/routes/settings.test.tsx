import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { expect, it } from "vitest";
import { adminLimits, settings } from "../test/fixtures";
import { renderWithSession } from "../test/render";
import { server } from "../test/server";
import { SettingsPage } from "./settings";

it("loads and saves runtime settings through the admin API", async () => {
  let patchBody: unknown;
  server.use(
    http.get("/api/admin/settings", () => HttpResponse.json(settings)),
    http.get("/api/admin/limits", () => HttpResponse.json(adminLimits)),
    http.patch("/api/admin/settings", async ({ request }) => {
      patchBody = await request.json();
      return HttpResponse.json(settings);
    })
  );
  const user = userEvent.setup();
  renderWithSession(<SettingsPage />);

  expect(await screen.findByText("Database runtime settings")).toBeInTheDocument();
  await user.type(screen.getByLabelText("Default timeout ms"), "4500");
  await user.click(screen.getByRole("button", { name: "Save settings" }));
  expect(await screen.findByText("Settings saved.")).toBeInTheDocument();
  expect(patchBody).toMatchObject({ default_request_timeout_ms: 4500 });
});
