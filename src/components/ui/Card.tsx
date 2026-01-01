import React, { forwardRef } from "react";
import { cn } from "@/lib/utils";

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {}

export const Card = forwardRef<HTMLDivElement, CardProps>(
  ({ className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn(
          "rounded-xl border border-gray-200 bg-white shadow-sm dark:border-gray-800 dark:bg-gray-900",
          className
        )}
        {...props}
      />
    );
  }
);
Card.displayName = "Card";

export const CardHeader = forwardRef<HTMLDivElement, CardProps>(
  ({ className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn("flex flex-col space-y-1.5 p-6", className)}
        {...props}
      />
    );
  }
);
CardHeader.displayName = "CardHeader";

export const CardTitle = forwardRef<
  HTMLHeadingElement,
  React.HTMLAttributes<HTMLHeadingElement>
>(({ className, ...props }, ref) => {
  return (
    <h3
      ref={ref}
      className={cn(
        "text-lg font-semibold leading-none tracking-tight",
        className
      )}
      {...props}
    />
  );
});
CardTitle.displayName = "CardTitle";

export const CardDescription = forwardRef<
  HTMLParagraphElement,
  React.HTMLAttributes<HTMLParagraphElement>
>(({ className, ...props }, ref) => {
  return (
    <p
      ref={ref}
      className={cn("text-sm text-gray-600 dark:text-gray-400", className)}
      {...props}
    />
  );
});
CardDescription.displayName = "CardDescription";

export const CardContent = forwardRef<HTMLDivElement, CardProps>(
  ({ className, ...props }, ref) => {
    return <div ref={ref} className={cn("p-6 pt-0", className)} {...props} />;
  }
);
CardContent.displayName = "CardContent";

export const CardFooter = forwardRef<HTMLDivElement, CardProps>(
  ({ className, ...props }, ref) => {
    return (
      <div
        ref={ref}
        className={cn("flex items-center p-6 pt-0", className)}
        {...props}
      />
    );
  }
);
CardFooter.displayName = "CardFooter";
