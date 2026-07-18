import { FileStack, ListChecks, NotebookText, Settings } from "lucide-react";
import type { ReactNode } from "react";
import type { AppPage } from "../contracts/desktop";
import { useAppStore } from "../stores/appStore";

interface AppShellProps {
  children: ReactNode;
}

const navigationItems: Array<{ page: Exclude<AppPage, "meeting-detail">; label: string; icon: typeof FileStack }> = [
  { page: "workspace", label: "转写工作台", icon: FileStack },
  { page: "tasks", label: "任务队列", icon: ListChecks },
  { page: "meetings", label: "会议记录", icon: NotebookText },
];

/** 提供稳定的企业桌面导航与主工作区布局。 */
export function AppShell({ children }: AppShellProps) {
  const page = useAppStore((state) => state.page);
  const navigate = useAppStore((state) => state.navigate);
  const openSettings = useAppStore((state) => state.openSettings);
  const settingsOpen = useAppStore((state) => state.settingsOpen);
  const taskAttentionCount = useAppStore((state) => state.taskAttentionCount);

  return (
    <div className="app-shell">
      <aside className="app-sidebar" aria-label="主导航">
        <div className="brand-block" aria-label="听见纪要">
          <span className="brand-mark" aria-hidden="true">听</span>
          <span className="brand-copy">
            <strong>听见纪要</strong>
            <small>音频工作台</small>
          </span>
        </div>

        <nav className="primary-navigation">
          {navigationItems.map((item) => {
            const Icon = item.icon;
            const isCurrent = item.page === page || (item.page === "meetings" && page === "meeting-detail");
            return (
              <button
                key={item.page}
                className="nav-item"
                type="button"
                aria-label={item.label}
                aria-current={isCurrent ? "page" : undefined}
                onClick={() => navigate(item.page)}
              >
                <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
                <span>{item.label}</span>
                {item.page === "tasks" && taskAttentionCount > 0 ? (
                  <span className="nav-badge" aria-label={`${taskAttentionCount} 个任务需要关注`}>{taskAttentionCount}</span>
                ) : null}
              </button>
            );
          })}
        </nav>

        <div className="sidebar-footer">
          <button className="nav-item" type="button" aria-label="设置" aria-expanded={settingsOpen} onClick={openSettings}>
            <Settings size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>设置</span>
          </button>
          <p>数据保存在本机，云端处理由你的配置决定。</p>
        </div>
      </aside>
      <main className="app-main" id="main-content">{children}</main>
    </div>
  );
}
