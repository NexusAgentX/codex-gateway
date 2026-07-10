import { Activity, ShieldAlert } from "lucide-react";
import { createBrowserRouter, Navigate, Outlet } from "react-router-dom";
import { AppShell } from "../components/layout/app-shell";
import { PageFrame } from "../components/layout/page-frame";
import { StateBlock, UnauthorizedState } from "../components/ui/state";
import { isAdmin, useSession } from "../lib/auth/session";
import { ApiKeysPage } from "../routes/api-keys";
import { LoginPage } from "../routes/login";
import { ModelsPage } from "../routes/models";
import { OverviewPage } from "../routes/overview";
import { RequestsPage } from "../routes/requests";
import { SettingsPage } from "../routes/settings";
import { UpstreamsPage } from "../routes/upstreams";
import { UsagePage } from "../routes/usage";
import { UsersPage } from "../routes/users";

function ProtectedRoute() {
  const { session, checkingSession } = useSession();

  if (checkingSession) {
    return (
      <div className="grid min-h-screen place-items-center p-5">
        <StateBlock>
          <Activity className="spin" size={20} />
          Checking session
        </StateBlock>
      </div>
    );
  }

  if (!session) {
    return <Navigate to="/login" replace />;
  }

  return <Outlet />;
}

function AdminRoute() {
  const { session } = useSession();
  if (session && isAdmin(session)) return <Outlet />;
  return (
    <PageFrame title="Admin only" icon={ShieldAlert}>
      <UnauthorizedState message="This page requires an admin account." />
    </PageFrame>
  );
}

export const router = createBrowserRouter([
  { path: "/login", element: <LoginPage /> },
  {
    element: <ProtectedRoute />,
    children: [
      {
        element: <AppShell />,
        children: [
          { index: true, element: <Navigate to="/overview" replace /> },
          { path: "/overview", element: <OverviewPage /> },
          { path: "/usage", element: <UsagePage /> },
          { path: "/requests", element: <RequestsPage /> },
          { path: "/api-keys", element: <ApiKeysPage /> },
          {
            element: <AdminRoute />,
            children: [
              { path: "/upstreams", element: <UpstreamsPage /> },
              { path: "/models", element: <ModelsPage /> },
              { path: "/users", element: <UsersPage /> },
              { path: "/settings", element: <SettingsPage /> }
            ]
          }
        ]
      }
    ]
  },
  { path: "*", element: <Navigate to="/overview" replace /> }
]);
