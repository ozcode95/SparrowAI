import { Accordion } from "../ui";
import ReactMarkdown from "react-markdown";

interface MessageSegment {
  type: "text" | "tool_call" | "tool_response" | "think";
  content: string;
  toolName?: string;
}

interface MessageContentProps {
  content: string;
}

function extractToolName(jsonContent: string): string | null {
  try {
    const parsed = JSON.parse(jsonContent);
    return parsed.name || null;
  } catch {
    return null;
  }
}

function cleanJsonString(str: string): string {
  try {
    // Try to parse as JSON first
    const parsed = JSON.parse(str);
    // Re-stringify with proper formatting
    return JSON.stringify(parsed, null, 2);
  } catch {
    // If not valid JSON, just clean up escape sequences
    return str
      .replace(/\\n/g, "\n")
      .replace(/\\"/g, '"')
      .replace(/\\\\/g, "\\");
  }
}

function parseMessageContent(content: string): MessageSegment[] {
  const segments: MessageSegment[] = [];
  let currentIndex = 0;

  const tagPatterns = [
    { type: "tool_call" as const, open: "<tool_call>", close: "</tool_call>" },
    {
      type: "tool_response" as const,
      open: "<tool_response>",
      close: "</tool_response>",
    },
    { type: "think" as const, open: "<think>", close: "</think>" },
  ];

  while (currentIndex < content.length) {
    let nearestTag: {
      index: number;
      type: "tool_call" | "tool_response" | "think";
      open: string;
      close: string;
    } | null = null;

    // Find the nearest opening tag
    for (const pattern of tagPatterns) {
      const index = content.indexOf(pattern.open, currentIndex);
      if (index !== -1 && (!nearestTag || index < nearestTag.index)) {
        nearestTag = { index, ...pattern };
      }
    }

    if (!nearestTag) {
      // No more tags, add remaining text
      const remainingText = content.substring(currentIndex).trim();
      if (remainingText) {
        segments.push({ type: "text", content: remainingText });
      }
      break;
    }

    // Add text before the tag
    const textBefore = content.substring(currentIndex, nearestTag.index).trim();
    if (textBefore) {
      segments.push({ type: "text", content: textBefore });
    }

    // Find the closing tag
    const closeIndex = content.indexOf(nearestTag.close, nearestTag.index);
    if (closeIndex === -1) {
      // No closing tag found - this is an incomplete tag (streaming)
      const incompleteContent = content
        .substring(nearestTag.index + nearestTag.open.length)
        .trim();

      // Extract tool name if available for incomplete tool_call
      const toolName =
        nearestTag.type === "tool_call"
          ? extractToolName(incompleteContent)
          : undefined;

      segments.push({
        type: nearestTag.type,
        content: incompleteContent + " ●", // Add loading indicator
        toolName,
      });
      break;
    }

    // Extract content between tags
    const tagContent = content
      .substring(nearestTag.index + nearestTag.open.length, closeIndex)
      .trim();

    // Extract tool name if it's a tool_call
    const toolName =
      nearestTag.type === "tool_call" ? extractToolName(tagContent) : undefined;

    segments.push({
      type: nearestTag.type,
      content: tagContent,
      toolName,
    });
    currentIndex = closeIndex + nearestTag.close.length;
  }

  return segments;
}

function getAccordionTitle(
  type: "tool_call" | "tool_response" | "think",
  toolName?: string
): string {
  let displayName = toolName;

  // Split tool name into server and function (format: server_function_name)
  if (toolName && toolName.includes("_")) {
    const parts = toolName.split("_");
    const serverName = parts[0];
    const functionName = parts.slice(1).join("_");
    displayName = `${serverName} → ${functionName}`;
  }

  switch (type) {
    case "tool_call":
      return displayName ? `🔧 Tool Call: ${displayName}` : "🔧 Tool Call";
    case "tool_response":
      return displayName
        ? `📋 Tool Response: ${displayName}`
        : "📋 Tool Response";
    case "think":
      return "💭 Reasoning";
  }
}

export const MessageContent: React.FC<MessageContentProps> = ({ content }) => {
  const segments = parseMessageContent(content);
  let lastToolName: string | undefined;

  return (
    <div className="space-y-2">
      {segments.map((segment, index) => {
        if (segment.type === "text") {
          return (
            <div
              key={index}
              className="prose prose-sm max-w-none dark:prose-invert"
            >
              <ReactMarkdown>{segment.content}</ReactMarkdown>
            </div>
          );
        }

        // Track tool name from tool_call for the next tool_response
        if (segment.type === "tool_call" && segment.toolName) {
          lastToolName = segment.toolName;
        }

        // Use the last tool name for tool_response
        const titleToolName =
          segment.type === "tool_response" ? lastToolName : segment.toolName;

        // Clean the content
        const cleanedContent = cleanJsonString(segment.content);

        return (
          <Accordion
            key={index}
            title={getAccordionTitle(segment.type, titleToolName)}
            variant={segment.type}
            defaultOpen={false}
          >
            <pre className="whitespace-pre-wrap font-mono text-xs overflow-x-auto">
              {cleanedContent}
            </pre>
          </Accordion>
        );
      })}
    </div>
  );
};
