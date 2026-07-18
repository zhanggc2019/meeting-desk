import { createContext, useContext, type ReactNode } from "react";
import type { DesktopClient } from "./desktopClient";

const DesktopClientContext = createContext<DesktopClient | null>(null);

interface DesktopClientProviderProps {
  client: DesktopClient;
  children: ReactNode;
}

/** 向所有业务页面提供同一桌面服务适配器实例。 */
export function DesktopClientProvider({ client, children }: DesktopClientProviderProps) {
  return <DesktopClientContext.Provider value={client}>{children}</DesktopClientContext.Provider>;
}

/** 获取当前桌面服务适配器，并在缺失 Provider 时快速失败。 */
export function useDesktopClient(): DesktopClient {
  const client = useContext(DesktopClientContext);
  if (!client) {
    throw new Error("DesktopClientProvider 未初始化");
  }
  return client;
}
