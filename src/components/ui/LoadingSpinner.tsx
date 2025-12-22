import React from "react";
import { cn } from "@/lib/utils";

export const LoadingSpinner: React.FC<{
  className?: string;
  size?: "sm" | "md" | "lg";
}> = ({ className, size = "md" }) => {
  const sizeClasses = {
    sm: "h-4 w-4 border-2",
    md: "h-6 w-6 border-2",
    lg: "h-8 w-8 border-3",
  };

  return (
    <div
      className={cn(
        "animate-spin rounded-full border-transparent border-t-accent-600 dark:border-t-accent-500",
        sizeClasses[size],
        className
      )}
    />
  );
};
