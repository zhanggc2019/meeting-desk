import { create } from "zustand";
import type { AppPage } from "../contracts/desktop";

interface AppState {
  page: AppPage;
  selectedMeetingId: string | null;
  settingsOpen: boolean;
  settingsRevision: number;
  taskAttentionCount: number;
  navigate: (page: AppPage) => void;
  openMeeting: (meetingId: string) => void;
  openSettings: () => void;
  closeSettings: () => void;
  markSettingsUpdated: () => void;
  setTaskAttentionCount: (count: number) => void;
}

/** 保存导航和抽屉等短生命周期 UI 状态，不持久化会议正文或密钥。 */
export const useAppStore = create<AppState>((set) => ({
  page: "workspace",
  selectedMeetingId: null,
  settingsOpen: false,
  settingsRevision: 0,
  taskAttentionCount: 0,
  navigate: (page) => set((state) => ({ page, selectedMeetingId: page === "meeting-detail" ? state.selectedMeetingId : null })),
  openMeeting: (selectedMeetingId) => set({ page: "meeting-detail", selectedMeetingId }),
  openSettings: () => set({ settingsOpen: true }),
  closeSettings: () => set({ settingsOpen: false }),
  markSettingsUpdated: () => set((state) => ({ settingsRevision: state.settingsRevision + 1 })),
  setTaskAttentionCount: (taskAttentionCount) => set({ taskAttentionCount }),
}));
