import { useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

export interface AccordionProps {
  title: string;
  children: React.ReactNode;
  defaultOpen?: boolean;
  variant?: "default" | "tool-call" | "tool-response" | "think";
}

export const Accordion: React.FC<AccordionProps> = ({
  title,
  children,
  defaultOpen = false,
  variant = "default",
}) => {
  const [isOpen, setIsOpen] = useState(defaultOpen);

  const variantStyles = {
    default: "bg-gray-50 dark:bg-gray-800 border-gray-200 dark:border-gray-700",
    "tool-call":
      "bg-blue-50 dark:bg-blue-900/20 border-blue-200 dark:border-blue-800",
    "tool-response":
      "bg-green-50 dark:bg-green-900/20 border-green-200 dark:border-green-800",
    think:
      "bg-purple-50 dark:bg-purple-900/20 border-purple-200 dark:border-purple-800",
  };

  return (
    <div
      className={cn(
        "border rounded-lg overflow-hidden mb-2",
        variantStyles[variant]
      )}
    >
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="w-full flex items-center justify-between p-3 text-left hover:opacity-80 transition-opacity"
      >
        <span className="font-medium text-sm">{title}</span>
        <ChevronDown
          className={cn(
            "w-4 h-4 transition-transform",
            isOpen && "transform rotate-180"
          )}
        />
      </button>
      {isOpen && (
        <div className="p-3 pt-0 text-sm">
          <div className="bg-white dark:bg-gray-900 rounded p-2 border border-gray-200 dark:border-gray-700">
            {children}
          </div>
        </div>
      )}
    </div>
  );
};
