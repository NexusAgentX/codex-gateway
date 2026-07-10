import { useMutation } from "@tanstack/react-query";
import { Activity, KeyRound, RefreshCw } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Navigate, useNavigate } from "react-router-dom";
import { Button } from "../components/ui/button";
import { Field, Input } from "../components/ui/form";
import { Notice } from "../components/ui/notice";
import { apiFetch } from "../lib/api/client";
import { messageForError } from "../lib/format";
import { useSession } from "../lib/auth/session";
import type { LoginResponse } from "../types/api";

export function LoginPage() {
  const { session, login } = useSession();
  const navigate = useNavigate();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");

  const mutation = useMutation({
    mutationFn: () =>
      apiFetch<LoginResponse>("/api/login", {
        method: "POST",
        body: { email, password }
      }),
    onSuccess(data) {
      login(data);
      navigate("/overview", { replace: true });
    }
  });

  if (session) {
    return <Navigate to="/overview" replace />;
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    mutation.mutate();
  }

  return (
    <div className="grid min-h-screen place-items-center p-5">
      <form className="grid w-[min(100%,380px)] gap-3 rounded-lg border border-zinc-200 bg-white p-5" onSubmit={submit}>
        <div className="mb-1 flex min-w-0 items-center gap-2 font-bold text-zinc-950">
          <Activity size={22} />
          <span className="truncate">codex-gateway</span>
        </div>
        <Field label="Email">
          <Input name="email" value={email} onChange={(event) => setEmail(event.target.value)} autoComplete="username" required />
        </Field>
        <Field label="Password">
          <Input name="password" value={password} onChange={(event) => setPassword(event.target.value)} type="password" autoComplete="current-password" required />
        </Field>
        {mutation.isError ? <Notice tone="error">{messageForError(mutation.error)}</Notice> : null}
        <Button type="submit" variant="primary" disabled={mutation.isPending}>
          {mutation.isPending ? <RefreshCw className="spin" size={16} /> : <KeyRound size={16} />}
          Sign in
        </Button>
      </form>
    </div>
  );
}
