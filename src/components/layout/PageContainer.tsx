import React from "react";
import { cn } from "@/lib/utils";

interface PageContainerProps {
  children: React.ReactNode;
  className?: string;
  title?: string;
  description?: string;
  actions?: React.ReactNode;
}

export const PageContainer: React.FC<PageContainerProps> = ({
  children,
  className,
  title,
  description,
  actions,
}) => {
  return (
    <div className={cn("flex h-full flex-col overflow-hidden", className)}>
      {(title || description || actions) && (
        <div className="mb-6 flex items-start justify-between">
          <div>
            {title && (
              <h1 className="text-2xl font-bold text-gray-900 dark:text-white">
                {title}
              </h1>
            )}
            {description && (
              <p className="mt-1 text-sm text-gray-600 dark:text-gray-400">
                {description}
              </p>
            )}
          </div>
          {actions && <div className="flex items-center gap-2">{actions}</div>}
        </div>
      )}
      <div className="flex-1 overflow-hidden">{children}</div>
    </div>
  );
};
