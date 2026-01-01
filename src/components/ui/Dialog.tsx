import React, { forwardRef } from "react";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface DialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: React.ReactNode;
}

export const Dialog: React.FC<DialogProps> = ({
  open,
  onOpenChange,
  children,
}) => {
  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="fixed inset-0 bg-black/50 backdrop-blur-sm"
        onClick={() => onOpenChange(false)}
      />
      <div className="relative z-50 animate-in">{children}</div>
    </div>
  );
};

export interface DialogContentProps
  extends React.HTMLAttributes<HTMLDivElement> {
  onClose?: () => void;
}

export const DialogContent = forwardRef<HTMLDivElement, DialogContentProps>(
  ({ className, children, onClose, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "relative w-full max-w-lg rounded-xl border border-gray-200 bg-white p-6 shadow-lg",
          "dark:border-gray-800 dark:bg-gray-900",
          className
        )}
        {...props}
      >
        {onClose && (
          <button
            onClick={onClose}
            className="absolute right-4 top-4 rounded-lg p-1 text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-900 dark:hover:bg-gray-800 dark:hover:text-gray-100"
          >
            <X className="h-4 w-4" />
          </button>
        )}
        {children}
      </div>
    );
  }
);
DialogContent.displayName = "DialogContent";

export const DialogHeader: React.FC<React.HTMLAttributes<HTMLDivElement>> = ({
  className,
  ...props
}) => (
  <div
    className={cn(
      "flex flex-col space-y-1.5 text-center sm:text-left",
      className
    )}
    {...props}
  />
);
DialogHeader.displayName = "DialogHeader";

export const DialogTitle: React.FC<
  React.HTMLAttributes<HTMLHeadingElement>
> = ({ className, ...props }) => (
  <h2
    className={cn(
      "text-lg font-semibold leading-none tracking-tight",
      className
    )}
    {...props}
  />
);
DialogTitle.displayName = "DialogTitle";

export const DialogDescription: React.FC<
  React.HTMLAttributes<HTMLParagraphElement>
> = ({ className, ...props }) => (
  <p
    className={cn("text-sm text-gray-600 dark:text-gray-400", className)}
    {...props}
  />
);
DialogDescription.displayName = "DialogDescription";

export const DialogFooter: React.FC<React.HTMLAttributes<HTMLDivElement>> = ({
  className,
  ...props
}) => (
  <div
    className={cn(
      "flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
      className
    )}
    {...props}
  />
);
DialogFooter.displayName = "DialogFooter";
