import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../test/server";
import { ApiClientError, apiFetch } from "./client";

describe("apiFetch", () => {
  it("sends bearer auth and JSON bodies", async () => {
    server.use(
      http.post("/api/example", async ({ request }) =>
        HttpResponse.json({
          authorization: request.headers.get("authorization"),
          contentType: request.headers.get("content-type"),
          body: await request.json()
        })
      )
    );

    await expect(
      apiFetch("/api/example", { method: "POST", token: "secret", body: { enabled: true } })
    ).resolves.toEqual({
      authorization: "Bearer secret",
      contentType: "application/json",
      body: { enabled: true }
    });
  });

  it("preserves structured gateway errors and supports empty responses", async () => {
    server.use(
      http.get("/api/failure", () =>
        HttpResponse.json(
          { error: { message: "admin role required", code: "forbidden" } },
          { status: 403 }
        )
      ),
      http.delete("/api/empty", () => new HttpResponse(null, { status: 204 }))
    );

    const error = await apiFetch("/api/failure").catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ApiClientError);
    expect(error).toMatchObject({ status: 403, code: "forbidden", message: "admin role required" });
    await expect(apiFetch("/api/empty", { method: "DELETE" })).resolves.toBeUndefined();
  });
});
