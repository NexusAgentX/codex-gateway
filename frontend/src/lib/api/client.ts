export class ApiClientError extends Error {
  status: number;
  code: string;

  constructor(status: number, message: string, code: string) {
    super(message);
    this.name = "ApiClientError";
    this.status = status;
    this.code = code;
  }
}

type ApiOptions = {
  method?: string;
  token?: string;
  body?: unknown;
};

export async function apiFetch<T>(path: string, options: ApiOptions = {}): Promise<T> {
  const headers = new Headers();
  if (options.token) {
    headers.set("Authorization", `Bearer ${options.token}`);
  }
  if (options.body !== undefined) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, {
    method: options.method ?? "GET",
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body)
  });

  if (!response.ok) {
    let message = response.statusText || "Request failed";
    let code = "request_failed";
    try {
      const payload = (await response.json()) as {
        error?: { message?: string; code?: string };
      };
      message = payload.error?.message ?? message;
      code = payload.error?.code ?? code;
    } catch {
      // Non-JSON failures can still be rendered with the HTTP status.
    }
    throw new ApiClientError(response.status, message, code);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}
