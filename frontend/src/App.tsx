import { useMemo } from "react";
import { AppShell } from "./components/AppShell";
import { MeetingDetailPage } from "./features/meetings/MeetingDetailPage";
import { MeetingsPage } from "./features/meetings/MeetingsPage";
import { WorkspacePage } from "./features/imports/WorkspacePage";
import { SettingsDrawer } from "./features/settings/SettingsDrawer";
import { TasksPage } from "./features/tasks/TasksPage";
import { UpdateBanner } from "./features/updates/UpdateBanner";
import { DesktopClientProvider } from "./services/DesktopClientContext";
import type { DesktopClient } from "./services/desktopClient";
import { createTauriDesktopClient, isTauriRuntime } from "./services/desktopClient";
import { createMockDesktopClient } from "./services/mockDesktopClient";
import type { UpdateService } from "./services/updateService";
import { createTauriUpdateService } from "./services/updateService";
import { useAppStore } from "./stores/appStore";

interface AppProps {
  client?: DesktopClient;
  updateService?: UpdateService | null;
}

/** 渲染当前导航状态对应的业务页面。 */
function CurrentPage() {
  const page = useAppStore((state) => state.page);
  if (page === "tasks") return <TasksPage />;
  if (page === "meetings") return <MeetingsPage />;
  if (page === "meeting-detail") return <MeetingDetailPage />;
  return <WorkspacePage />;
}

/** 组装 Tauri 或浏览器测试服务边界与完整桌面工作台。 */
export function App({ client, updateService }: AppProps) {
  const resolvedClient = useMemo(
    () => client ?? (isTauriRuntime() ? createTauriDesktopClient() : createMockDesktopClient()),
    [client],
  );
  const resolvedUpdateService = useMemo(
    () => updateService === undefined
      ? (isTauriRuntime() ? createTauriUpdateService() : null)
      : updateService,
    [updateService],
  );

  return (
    <DesktopClientProvider client={resolvedClient}>
      <AppShell><UpdateBanner service={resolvedUpdateService} /><CurrentPage /></AppShell>
      <SettingsDrawer />
    </DesktopClientProvider>
  );
}
