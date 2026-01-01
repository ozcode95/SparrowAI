import React, { forwardRef } from "react";
import { cn } from "@/lib/utils";

export interface InputProps
  extends React.InputHTMLAttributes<HTMLInputElement> {}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        className={cn(
          "flex h-10 w-full rounded-lg border border-gray-300 bg-white px-3 py-2 text-sm transition-colors",
          "placeholder:text-gray-400",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-500 focus-visible:ring-offset-0",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "dark:border-gray-700 dark:bg-gray-900 dark:placeholder:text-gray-600",
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Input.displayName = "Input";

export interface TextareaProps
  extends React.TextareaHTMLAttributes<HTMLTextAreaElement> {}

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          "flex min-h-[80px] w-full rounded-lg border border-gray-300 bg-white px-3 py-2 text-sm transition-colors",
          "placeholder:text-gray-400",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-500 focus-visible:ring-offset-0",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "dark:border-gray-700 dark:bg-gray-900 dark:placeholder:text-gray-600",
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Textarea.displayName = "Textarea";
