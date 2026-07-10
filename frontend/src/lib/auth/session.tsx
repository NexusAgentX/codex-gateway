import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { ApiClientError, apiFetch } from "../api/client";
import type { LoginResponse, LoginUser } from "../../types/api";

export type Session = {
  token: string;
  user: LoginUser;
};

type SessionContextValue = {
  session: Session | null;
  checkingSession: boolean;
  login: (login: LoginResponse) => void;
  logout: () => void;
};

const sessionKey = "codex-gateway-session";
const SessionContext = createContext<SessionContextValue | null>(null);

export function SessionProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(() => readStoredSession());
  const [checkingSession, setCheckingSession] = useState(Boolean(session));

  useEffect(() => {
    if (!session) {
      setCheckingSession(false);
      return;
    }
    let active = true;
    apiFetch<{ email: string; role: string; user_id: string }>("/api/me", { token: session.token })
      .then((me) => {
        if (!active) return;
        const next = {
          ...session,
          user: { id: me.user_id, email: me.email, role: me.role }
        };
        setSession(next);
        storeSession(next);
      })
      .catch((error) => {
        if (!active) return;
        if (error instanceof ApiClientError && error.status === 401) {
          clearStoredSession();
          setSession(null);
        }
      })
      .finally(() => active && setCheckingSession(false));
    return () => {
      active = false;
    };
  }, []);

  const value = useMemo<SessionContextValue>(
    () => ({
      session,
      checkingSession,
      login(loginResponse) {
        const next = { token: loginResponse.token, user: loginResponse.user };
        storeSession(next);
        setSession(next);
      },
      logout() {
        clearStoredSession();
        setSession(null);
      }
    }),
    [checkingSession, session]
  );

  return <SessionContext.Provider value={value}>{children}</SessionContext.Provider>;
}

export function useSession() {
  const value = useContext(SessionContext);
  if (!value) {
    throw new Error("useSession must be used inside SessionProvider");
  }
  return value;
}

export function isAdmin(session: Session) {
  return session.user.role === "admin";
}

function readStoredSession(): Session | null {
  const raw = localStorage.getItem(sessionKey);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Session;
  } catch {
    clearStoredSession();
    return null;
  }
}

function storeSession(session: Session) {
  localStorage.setItem(sessionKey, JSON.stringify(session));
}

function clearStoredSession() {
  localStorage.removeItem(sessionKey);
}
