import React from "react";
import { useUI } from "@/store";
import { cn } from "@/lib/utils";

const DRAWER_WIDTH = 240;
const DRAWER_WIDTH_COLLAPSED = 64;

interface AppLayoutProps {
  sidebar: React.ReactNode;
  children: React.ReactNode;
}

export const AppLayout: React.FC<AppLayoutProps> = ({ sidebar, children }) => {
  const { sidebarCollapsed } = useUI();

  const drawerWidth = sidebarCollapsed ? DRAWER_WIDTH_COLLAPSED : DRAWER_WIDTH;

  return (
    <div className="flex h-screen overflow-hidden bg-gray-50 dark:bg-gray-900">
      {sidebar}

      <main
        className={cn(
          "flex-1 overflow-hidden transition-all duration-300",
          "flex flex-col"
        )}
        style={{
          width: `calc(100% - ${drawerWidth}px)`,
        }}
      >
        <div className="mx-auto flex h-full w-full max-w-full flex-col overflow-hidden p-4 sm:p-6">
          {children}
        </div>
      </main>
    </div>
  );
};
