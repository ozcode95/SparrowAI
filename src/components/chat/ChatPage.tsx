import { useState, useEffect, useRef } from "react";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PageContainer } from "../layout";
import { Card, Button, Input } from "../ui";
import { MessageContent } from "./MessageContent";
import { useAppStore } from "@/store";
import {
  Send,
  User,
  Loader2,
  Wrench,
  StopCircle,
  Paperclip,
  X,
} from "lucide-react";
import ReactMarkdown from "react-markdown";
import type { ChatMessage as ChatMessageType } from "@/store/types";
import { categorizeModel } from "@/lib/modelUtils";
import {
  logUserAction,
  logError,
  logInfo,
  logDebug,
  logWarn,
} from "@/lib/logger";

interface ToolCall {
  tool_name: string;
  arguments: string;
  result: string;
}

interface AttachmentInfo {
  file_path: string;
  file_name: string;
  file_type: string;
  is_image?: boolean;
}

export const ChatPage = () => {
  const {
    activeChatSessionId,
    currentChatMessages,
    addMessageToCurrentChat,
    setCurrentChatMessages,
    temporarySession,
    setActiveChatSessionId,
  } = useAppStore();
  const { settings } = useAppStore();
  const { downloadedModels, loadedModelsByType, setLoadedModelByType } =
    useAppStore();
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [currentStreamingMessage, setCurrentStreamingMessage] = useState("");
  const [, setToolCalls] = useState<ToolCall[]>([]);
  const [, setUsageData] = useState<{
    promptTokens: number;
    completionTokens: number;
    totalTokens: number;
  } | null>(null);
  const [isLoadingModel, setIsLoadingModel] = useState<{
    text: boolean;
    "image-to-text": boolean;
  }>({
    text: false,
    "image-to-text": false,
  });

  // Local LLM configuration (optional overrides)
  const [localTemperature, setLocalTemperature] = useState<number | null>(null);
  const [localTopP, setLocalTopP] = useState<number | null>(null);
  const [localMaxTokens, setLocalMaxTokens] = useState<number | null>(null);
  const [showConfigPanel, setShowConfigPanel] = useState(false);
  const [attachments, setAttachments] = useState<AttachmentInfo[]>([]);
  const [showAttachmentPanel, setShowAttachmentPanel] = useState(false);
  const [showFileSelector, setShowFileSelector] = useState(false);
  const [availableFiles, setAvailableFiles] = useState<
    Array<{
      file_path: string;
      file_name: string;
      file_type: string;
      chunk_count: number;
    }>
  >([]);
  const [isLoadingFiles, setIsLoadingFiles] = useState(false);
  const [isUploadingFile, setIsUploadingFile] = useState(false);
  const [fileSearchQuery, setFileSearchQuery] = useState("");

  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Organize downloaded models by category using state instead of ref
  const [modelsByCategory, setModelsByCategory] = useState<{
    text: string[];
    "image-to-text": string[];
  }>({
    text: [],
    "image-to-text": [],
  });

  // Update categorized models when downloadedModels changes
  useEffect(() => {
    console.log(
      "[ChatPage] downloadedModels changed:",
      downloadedModels.size,
      Array.from(downloadedModels)
    );

    const categorized: {
      text: string[];
      "image-to-text": string[];
    } = {
      text: [],
      "image-to-text": [],
    };

    Array.from(downloadedModels).forEach((modelId) => {
      const category = categorizeModel(modelId);
      console.log(`[ChatPage] Categorizing ${modelId} as ${category}`);
      if (category === "text" || category === "image-to-text") {
        categorized[category].push(modelId);
      }
    });

    setModelsByCategory(categorized);
    console.log("[ChatPage] modelsByCategory updated:", categorized);
  }, [downloadedModels]);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [currentChatMessages, currentStreamingMessage]);

  const loadAvailableFiles = async () => {
    setIsLoadingFiles(true);
    try {
      const files = await invoke<
        Array<{
          file_path: string;
          file_name: string;
          file_type: string;
          chunk_count: number;
        }>
      >("get_all_files");
      setAvailableFiles(files);
    } catch (error) {
      logError("Failed to load files", error as Error);
    } finally {
      setIsLoadingFiles(false);
    }
  };

  const handleFileUpload = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: true,
        filters: [
          {
            name: "Documents & Images",
            extensions: [
              "pdf",
              "docx",
              "xlsx",
              "xls",
              "png",
              "jpg",
              "jpeg",
              "webp",
              "gif",
            ],
          },
        ],
      });

      if (!selected || (Array.isArray(selected) && selected.length === 0)) {
        return;
      }

      const filePaths = Array.isArray(selected) ? selected : [selected];
      setIsUploadingFile(true);

      for (const filePath of filePaths) {
        try {
          // Check if file is an image
          const extension = filePath.split(".").pop()?.toLowerCase() || "";
          const isImage = ["png", "jpg", "jpeg", "webp", "gif"].includes(
            extension
          );

          if (isImage) {
            // Images don't go through RAG processing, just add directly to attachments
            logInfo(`Added image file ${filePath} (skipping RAG processing)`);
            const fileName = filePath.split(/[\\/]/).pop() || filePath;
            setAttachments((prev) => [
              ...prev,
              {
                file_path: filePath,
                file_name: fileName,
                file_type: extension,
                is_image: true,
              },
            ]);
          } else {
            // Process document into chunks for RAG
            const documents = await invoke<any[]>("process_document", {
              filePath,
            });
            const embeddedDocs = await invoke<any[]>(
              "create_document_embeddings",
              { documents }
            );
            await invoke("store_documents", { documents: embeddedDocs });
            logInfo(`Successfully processed and stored ${filePath}`);
          }
        } catch (error) {
          logError(`Failed to process ${filePath}`, error as Error);
          alert(`Failed to process ${filePath.split("\\").pop()}: ${error}`);
        }
      }

      // Reload files if we're in the file selector view
      if (showFileSelector) {
        await loadAvailableFiles();
      }
      // Close the panel after upload
      setShowAttachmentPanel(false);
      setShowFileSelector(false);
    } catch (error) {
      console.error("Upload failed:", error);
    } finally {
      setIsUploadingFile(false);
    }
  };

  const toggleAttachment = (filePath: string) => {
    setAttachments((prev) => {
      // Check if already attached
      const existingIndex = prev.findIndex((a) => a.file_path === filePath);

      if (existingIndex !== -1) {
        // Remove it
        return prev.filter((a) => a.file_path !== filePath);
      } else {
        // Add it
        const extension = filePath.split(".").pop()?.toLowerCase() || "";
        const isImage = ["png", "jpg", "jpeg", "webp", "gif"].includes(
          extension
        );
        const fileName = filePath.split(/[\\/]/).pop() || filePath;

        return [
          ...prev,
          {
            file_path: filePath,
            file_name: fileName,
            file_type: extension,
            is_image: isImage,
          },
        ];
      }
    });
  };

  // Clear input when session changes (new chat)
  useEffect(() => {
    setInput("");
  }, [activeChatSessionId, temporarySession]);

  // Load messages for active session (but only when switching sessions, not when creating new)
  const previousSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    // Only load messages if session changed AND it's not a new session we just created
    if (activeChatSessionId && !temporarySession) {
      // Skip loading if this is the first time setting the session (new session)
      if (
        previousSessionIdRef.current === null &&
        currentChatMessages.length > 0
      ) {
        previousSessionIdRef.current = activeChatSessionId;
        return;
      }

      // Skip if session hasn't changed
      if (previousSessionIdRef.current === activeChatSessionId) {
        return;
      }

      previousSessionIdRef.current = activeChatSessionId;

      invoke<ChatMessageType[]>("get_session_messages", {
        sessionId: activeChatSessionId,
      })
        .then((messages) => {
          setCurrentChatMessages(messages);
        })
        .catch((error) => {
          logError("Failed to load session messages", error as Error, {
            sessionId: activeChatSessionId,
          });
        });
    } else if (!activeChatSessionId) {
      previousSessionIdRef.current = null;
    }
  }, [
    activeChatSessionId,
    temporarySession,
    setCurrentChatMessages,
    currentChatMessages.length,
  ]);

  // Listen for streaming tokens
  useEffect(() => {
    let accumulatedMessage = "";
    let accumulatedToolCalls: ToolCall[] = [];
    let streamStartTime: number | null = null;

    const unlisten = listen<{
      token: string;
      finished: boolean;
      cancelled?: boolean;
      usage?: {
        prompt_tokens: number;
        completion_tokens: number;
        total_tokens: number;
      };
    }>("chat-token", async (event) => {
      if (event.payload.finished) {
        logDebug("Chat streaming finished", {
          messageLength: accumulatedMessage.length,
          cancelled: event.payload.cancelled,
        });

        // Extract usage data from the finished event payload
        const usageFromPayload = event.payload.usage
          ? {
              promptTokens: event.payload.usage.prompt_tokens,
              completionTokens: event.payload.usage.completion_tokens,
              totalTokens: event.payload.usage.total_tokens,
            }
          : null;

        console.log("Usage data from finished event:", usageFromPayload);

        // Calculate tokens per second
        let tokensPerSecond: number | null = null;
        if (usageFromPayload && streamStartTime) {
          const elapsedSeconds = (Date.now() - streamStartTime) / 1000;
          tokensPerSecond = usageFromPayload.completionTokens / elapsedSeconds;
          logInfo("Chat stream completed", {
            tokensPerSecond: tokensPerSecond.toFixed(2),
            completionTokens: usageFromPayload.completionTokens,
            elapsedSeconds: elapsedSeconds.toFixed(2),
          });
        }

        // Save the complete assistant message to chat session
        if (accumulatedMessage.trim()) {
          try {
            // Save to backend
            if (activeChatSessionId) {
              logDebug("Saving assistant message", {
                sessionId: activeChatSessionId,
                hasUsageData: !!usageFromPayload,
                cancelled: event.payload.cancelled,
              });
              await invoke("add_message_to_session", {
                sessionId: activeChatSessionId,
                role: "assistant",
                content: accumulatedMessage,
                tokensPerSecond: tokensPerSecond,
                isError: false,
                promptTokens: usageFromPayload?.promptTokens ?? null,
                completionTokens: usageFromPayload?.completionTokens ?? null,
                totalTokens: usageFromPayload?.totalTokens ?? null,
              });
              logInfo("Assistant message saved", {
                sessionId: activeChatSessionId,
                contentLength: accumulatedMessage.length,
                cancelled: event.payload.cancelled,
              });

              // Add to UI with usage data
              const assistantMessage: ChatMessageType = {
                id: crypto.randomUUID(),
                role: "assistant",
                content: accumulatedMessage,
                timestamp: Date.now(),
                tokens_per_second: tokensPerSecond,
                completion_tokens: usageFromPayload?.completionTokens,
                total_tokens: usageFromPayload?.totalTokens,
                prompt_tokens: usageFromPayload?.promptTokens,
                toolCalls:
                  accumulatedToolCalls.length > 0
                    ? accumulatedToolCalls
                    : undefined,
              };
              addMessageToCurrentChat(assistantMessage);
            } else {
              logWarn("No active session ID to save message");
            }
          } catch (error) {
            logError("Failed to save assistant message", error as Error, {
              sessionId: activeChatSessionId,
            });
          }
        }

        // Clear streaming state
        setCurrentStreamingMessage("");
        setToolCalls([]);
        setIsStreaming(false);
        setUsageData(null); // Clear usage data for next message
        accumulatedMessage = "";
        accumulatedToolCalls = [];
        streamStartTime = null;
      } else {
        // Set start time on first token
        if (!streamStartTime) {
          streamStartTime = Date.now();
        }

        // Accumulate the token
        accumulatedMessage += event.payload.token;
        setCurrentStreamingMessage((prev) => prev + event.payload.token);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [activeChatSessionId, addMessageToCurrentChat]);

  // Listen for tool calls
  useEffect(() => {
    const unlisten = listen<ToolCall>("tool-call", (event) => {
      setToolCalls((prev) => [...prev, event.payload]);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for usage statistics
  useEffect(() => {
    const unlisten = listen<{
      prompt_tokens: number;
      completion_tokens: number;
      total_tokens: number;
    }>("chat-usage", (event) => {
      logDebug("Received usage statistics", event.payload);
      setUsageData({
        promptTokens: event.payload.prompt_tokens,
        completionTokens: event.payload.completion_tokens,
        totalTokens: event.payload.total_tokens,
      });
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for chat errors
  useEffect(() => {
    const unlisten = listen<{ error: string }>("chat-error", (event) => {
      setIsStreaming(false);
      setCurrentStreamingMessage("");
      logError("Chat error occurred", new Error(event.payload.error));

      const errorMessage: ChatMessageType = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: `Error: ${event.payload.error}`,
        timestamp: Date.now(),
      };
      addMessageToCurrentChat(errorMessage);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [addMessageToCurrentChat]);

  const handleSend = async () => {
    if (!input.trim() || isStreaming) return;

    // Check if images are attached
    const hasImageAttachments = attachments.some((a) => a.is_image);

    // Check if vision model is loaded when images are attached
    if (hasImageAttachments && !loadedModelsByType["image-to-text"]) {
      alert("Please select a Vision (Image-to-Text) model to process images");
      return;
    }

    // Check if text model is loaded (required for chat)
    if (!loadedModelsByType.text) {
      alert("Please select a Text Generation model first");
      return;
    }

    const currentAttachments = [...attachments]; // Store current attachments
    const userMessage: ChatMessageType = {
      id: crypto.randomUUID(),
      role: "user",
      content: input.trim(),
      timestamp: Date.now(),
      attachments:
        currentAttachments.length > 0 ? currentAttachments : undefined,
    };

    // Add user message to UI immediately
    addMessageToCurrentChat(userMessage);

    const messageContent = input.trim();
    setInput("");
    setAttachments([]); // Clear attachments after sending
    setIsStreaming(true);
    setToolCalls([]);

    let sessionToUse = activeChatSessionId; // Declare outside try block

    try {
      console.log("handleSend - activeChatSessionId:", activeChatSessionId);
      console.log("handleSend - temporarySession:", temporarySession);

      // Create new session if this is the first message (no active session)
      if (!activeChatSessionId && !temporarySession) {
        logInfo("Creating new chat session");
        const newSession = await invoke<any>("create_chat_session", {
          title: null,
        });
        logInfo("New session created", { sessionId: newSession.id });
        sessionToUse = newSession.id;
        setActiveChatSessionId(newSession.id);

        // Add user message to the new session
        logDebug("Adding user message to new session", {
          sessionId: newSession.id,
        });
        await invoke("add_message_to_session", {
          sessionId: newSession.id,
          role: "user",
          content: messageContent,
          tokensPerSecond: null,
          isError: false,
          attachments:
            currentAttachments.length > 0 ? currentAttachments : null,
        });
        logDebug("User message added to new session");

        // Trigger sidebar refresh
        window.dispatchEvent(new CustomEvent("chat-session-created"));
      } else if (activeChatSessionId) {
        // Verify the session exists in backend before adding message
        logDebug("Verifying session exists", {
          sessionId: activeChatSessionId,
        });
        try {
          await invoke("get_session_messages", {
            sessionId: activeChatSessionId,
          });
          logDebug("Session verified, adding message", {
            sessionId: activeChatSessionId,
          });

          // Add to existing active session
          await invoke("add_message_to_session", {
            sessionId: activeChatSessionId,
            role: "user",
            content: messageContent,
            tokensPerSecond: null,
            isError: false,
            attachments:
              currentAttachments.length > 0 ? currentAttachments : null,
          });
          logDebug("User message added to existing session", {
            sessionId: activeChatSessionId,
          });
        } catch (error) {
          logWarn("Session not found in backend, creating new one", {
            sessionId: activeChatSessionId,
          });
          // Session doesn't exist in backend, create a new one
          const newSession = await invoke<any>("create_chat_session", {
            title: null,
          });
          logInfo("New session created", { sessionId: newSession.id });
          sessionToUse = newSession.id;
          setActiveChatSessionId(newSession.id);

          // Add user message to the new session
          await invoke("add_message_to_session", {
            sessionId: newSession.id,
            role: "user",
            content: messageContent,
            tokensPerSecond: null,
            isError: false,
            attachments:
              currentAttachments.length > 0 ? currentAttachments : null,
          });
          logDebug("User message added to new session");

          // Trigger sidebar refresh
          window.dispatchEvent(new CustomEvent("chat-session-created"));
        }
      }

      logDebug("Starting chat with session", {
        sessionId: sessionToUse,
        modelId: loadedModelsByType.text,
      });
      logUserAction("Send chat message", {
        sessionId: sessionToUse,
        messageLength: messageContent.length,
      });

      // Prepare LLM configuration (use local overrides if set, otherwise global settings)
      const effectiveTemperature =
        localTemperature !== null ? localTemperature : settings.temperature;
      const effectiveTopP = localTopP !== null ? localTopP : settings.topP;
      const effectiveMaxTokens =
        localMaxTokens !== null ? localMaxTokens : settings.maxTokens;

      // Start streaming chat with all configuration
      // Use vision model if images are attached, otherwise use text model
      const hasImageAttachments = currentAttachments.some((a) => a.is_image);
      const modelToUse = hasImageAttachments
        ? loadedModelsByType["image-to-text"]!
        : loadedModelsByType.text!;

      // Strip 'OpenVINO/' prefix from model name if present
      const modelNameForChat = modelToUse.startsWith("OpenVINO/")
        ? modelToUse.substring("OpenVINO/".length)
        : modelToUse;

      // Choose the appropriate chat function based on RAG setting
      const chatCommand = settings.useRAG
        ? "chat_with_rag_streaming"
        : "chat_with_loaded_model_streaming";

      logDebug(`Using chat command: ${chatCommand}`, {
        useRAG: settings.useRAG,
        hasAttachments: currentAttachments.length > 0,
      });

      await invoke(chatCommand, {
        sessionId: sessionToUse,
        message: messageContent,
        modelName: modelNameForChat,
        includeHistory: settings.includeConversationHistory,
        systemPrompt: settings.systemPrompt || null,
        temperature: effectiveTemperature,
        topP: effectiveTopP,
        seed: settings.seed,
        maxTokens: effectiveMaxTokens,
        maxCompletionTokens: settings.maxCompletionTokens,
        // RAG-specific parameters (only used if chatCommand is chat_with_rag_streaming)
        useRag: settings.useRAG || currentAttachments.some((a) => !a.is_image), // Use RAG if enabled OR if there are document attachments
        ragLimit: currentAttachments.some((a) => !a.is_image) ? null : 5, // null means use only attached documents
        // Pass full attachment objects (includes both images and documents)
        attachments: currentAttachments.length > 0 ? currentAttachments : null,
      });
    } catch (error) {
      logError("Failed to send message", error as Error, {
        sessionId: sessionToUse,
      });
      setIsStreaming(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleStop = async () => {
    if (!isStreaming) return;

    try {
      logUserAction("Stopping chat streaming");

      // Get the active session ID
      const sessionToStop = activeChatSessionId || "temp";

      await invoke("stop_chat_streaming", {
        sessionId: sessionToStop,
      });

      logInfo("Chat streaming stopped", { sessionId: sessionToStop });
      setIsStreaming(false);
      setCurrentStreamingMessage("");
    } catch (error) {
      logError("Failed to stop streaming", error as Error);
    }
  };

  const handleLoadModel = async (
    modelId: string,
    modelType: "text" | "image-to-text"
  ) => {
    setIsLoadingModel((prev) => ({ ...prev, [modelType]: true }));
    try {
      console.log(`Loading model: ${modelId} (type: ${modelType})`);
      await invoke("load_model", { modelId });
      setLoadedModelByType(modelType, modelId);

      console.log(`Model loaded successfully: ${modelId}`);
    } catch (error) {
      console.error("Failed to load model:", error);
      logError("Failed to load model", error as Error, { modelId, modelType });
      alert(`Failed to load model: ${error}`);
    } finally {
      setIsLoadingModel((prev) => ({ ...prev, [modelType]: false }));
    }
  };

  const isAnyModelLoading =
    isLoadingModel.text || isLoadingModel["image-to-text"];

  return (
    <PageContainer
      title="Chat"
      description="Conversation with AI assistant"
      actions={
        <div className="flex items-center gap-3 py-1 pr-2">
          {/* Text Generation Model */}
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-gray-600 dark:text-gray-400 px-1">
              Text Generation *
            </label>
            <select
              value={loadedModelsByType.text || ""}
              onChange={(e) =>
                e.target.value && handleLoadModel(e.target.value, "text")
              }
              disabled={isAnyModelLoading || isStreaming}
              className="px-2 py-1.5 border rounded-lg bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600 text-xs disabled:opacity-50 disabled:cursor-not-allowed w-56 truncate"
            >
              <option value="">Select model...</option>
              {modelsByCategory.text.map((modelId) => (
                <option key={modelId} value={modelId}>
                  {modelId}
                </option>
              ))}
            </select>
          </div>

          {/* Image-to-Text Model */}
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-gray-600 dark:text-gray-400 px-1">
              Vision (Image-to-Text)
            </label>
            <select
              value={loadedModelsByType["image-to-text"] || ""}
              onChange={(e) =>
                e.target.value &&
                handleLoadModel(e.target.value, "image-to-text")
              }
              disabled={isAnyModelLoading || isStreaming}
              className="px-2 py-1.5 border rounded-lg bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600 text-xs disabled:opacity-50 disabled:cursor-not-allowed w-56 truncate"
            >
              <option value="">Select model...</option>
              {modelsByCategory["image-to-text"].map((modelId) => (
                <option key={modelId} value={modelId}>
                  {modelId}
                </option>
              ))}
            </select>
          </div>

          {/* Loading indicators */}
          {(isLoadingModel.text || isLoadingModel["image-to-text"]) && (
            <Loader2 className="w-4 h-4 animate-spin text-gray-400" />
          )}
        </div>
      }
    >
      <div className="flex flex-col h-full">
        {/* Messages Area */}
        <div className="flex-1 overflow-y-auto mb-4 space-y-4 px-2">
          {currentChatMessages.length === 0 && !isStreaming && (
            <Card className="p-8 text-center">
              <img
                src="/white_logo_xbg.png"
                alt="SparrowAI"
                className="w-16 h-16 mx-auto mb-4"
              />
              <h2 className="text-xl font-semibold mb-2">
                Start a conversation
              </h2>
              <p className="text-gray-600 dark:text-gray-400">
                {loadedModelsByType.text
                  ? `Using ${loadedModelsByType.text}. Type a message below to begin.`
                  : "Load a Text Generation model above to start chatting."}
              </p>
            </Card>
          )}

          {currentChatMessages.map((message) => (
            <div
              key={message.id}
              className={`flex gap-3 items-center ${
                message.role === "user" ? "justify-end" : "justify-start"
              }`}
            >
              {message.role === "assistant" && (
                <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center p-1">
                  <img
                    src="/white_logo_xbg.png"
                    alt="AI"
                    className="w-full h-full object-contain"
                  />
                </div>
              )}

              <Card
                className={`${
                  message.role === "user"
                    ? "bg-primary text-white max-w-[85%]"
                    : "flex-1"
                } p-4`}
              >
                {message.role === "assistant" ? (
                  <>
                    <MessageContent content={message.content} />
                    {/* Display tokens per second and usage info */}
                    {(message.tokens_per_second || message.total_tokens) && (
                      <div className="mt-3 pt-2 border-t border-gray-200 dark:border-gray-700 text-xs text-gray-500 dark:text-gray-400 flex items-center gap-4">
                        {message.tokens_per_second && (
                          <span className="font-mono">
                            ⚡ {message.tokens_per_second.toFixed(2)} tokens/s
                          </span>
                        )}
                        {message.total_tokens && (
                          <span className="font-mono">
                            📊 {message.completion_tokens || 0} tokens
                          </span>
                        )}
                      </div>
                    )}
                  </>
                ) : (
                  <div className="prose prose-sm max-w-none dark:prose-invert">
                    <ReactMarkdown>{message.content}</ReactMarkdown>
                  </div>
                )}

                {/* Display attachments */}
                {message.attachments && message.attachments.length > 0 && (
                  <div
                    className={`mt-3 pt-3 ${
                      message.role === "user"
                        ? "border-t border-white/20"
                        : "border-t border-gray-200 dark:border-gray-700"
                    }`}
                  >
                    <div className="flex items-center gap-2 mb-2 text-xs opacity-70">
                      <Paperclip className="w-3 h-3" />
                      <span>
                        Attached{" "}
                        {message.attachments.some((a) => a.is_image)
                          ? "documents & images"
                          : "documents"}
                        :
                      </span>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      {message.attachments.map((attachment, idx) =>
                        attachment.is_image ? (
                          <div
                            key={idx}
                            className={`rounded overflow-hidden ${
                              message.role === "user"
                                ? "border border-white/20"
                                : "border border-gray-200 dark:border-gray-700"
                            }`}
                          >
                            <img
                              src={convertFileSrc(attachment.file_path)}
                              alt={attachment.file_name}
                              className="max-w-[200px] max-h-[200px] object-cover"
                              title={attachment.file_name}
                            />
                          </div>
                        ) : (
                          <div
                            key={idx}
                            className={`text-xs px-2 py-1 rounded flex items-center gap-1.5 ${
                              message.role === "user"
                                ? "bg-white/20"
                                : "bg-gray-100 dark:bg-gray-800"
                            }`}
                          >
                            <Paperclip className="w-3 h-3" />
                            <span className="max-w-[150px] truncate">
                              {attachment.file_name}
                            </span>
                          </div>
                        )
                      )}
                    </div>
                  </div>
                )}

                {message.toolCalls && message.toolCalls.length > 0 && (
                  <div className="mt-3 pt-3 border-t border-gray-200 dark:border-gray-700 space-y-2">
                    {message.toolCalls.map((tool, idx) => (
                      <div key={idx} className="text-xs space-y-1">
                        <div className="flex items-center gap-2 text-accent">
                          <Wrench className="w-3 h-3" />
                          <span className="font-medium">{tool.tool_name}</span>
                        </div>
                        <div className="pl-5 text-gray-600 dark:text-gray-400">
                          <div className="font-mono bg-gray-100 dark:bg-gray-800 rounded px-2 py-1">
                            {tool.result}
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </Card>

              {message.role === "user" && (
                <div className="flex-shrink-0 w-8 h-8 rounded-full bg-accent/10 flex items-center justify-center">
                  <User className="w-5 h-5 text-accent" />
                </div>
              )}
            </div>
          ))}

          {/* Streaming message */}
          {isStreaming && currentStreamingMessage && (
            <div className="flex gap-3 items-center justify-start">
              <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center p-1">
                <img
                  src="/white_logo_xbg.png"
                  alt="AI"
                  className="w-full h-full object-contain"
                />
              </div>
              <Card className="flex-1 p-4">
                <MessageContent content={currentStreamingMessage} />
              </Card>
            </div>
          )}

          {/* Loading indicator */}
          {isStreaming && !currentStreamingMessage && (
            <div className="flex gap-3 items-center justify-start">
              <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center relative p-1">
                <div className="absolute inset-0 rounded-full border-2 border-transparent border-t-primary animate-spin"></div>
                <img
                  src="/white_logo_xbg.png"
                  alt="AI"
                  className="w-full h-full object-contain"
                />
              </div>
              <Card className="p-4">
                <div className="flex items-center gap-2 text-gray-600 dark:text-gray-400">
                  <Loader2 className="w-4 h-4 animate-spin" />
                  <span className="text-sm">Thinking...</span>
                </div>
              </Card>
            </div>
          )}

          <div ref={messagesEndRef} />
        </div>

        {/* Input Area */}
        <Card className="p-4">
          <div className="flex flex-col gap-3">
            {/* Optional: Local LLM Config Panel */}
            {showConfigPanel && (
              <div className="flex flex-wrap gap-3 pb-3 border-b border-gray-200 dark:border-gray-700">
                <div className="flex flex-col gap-1">
                  <label className="text-xs text-gray-600 dark:text-gray-400">
                    Temperature
                  </label>
                  <input
                    type="number"
                    min="0"
                    max="2"
                    step="0.1"
                    value={
                      localTemperature !== null
                        ? localTemperature
                        : settings.temperature ?? ""
                    }
                    onChange={(e) =>
                      setLocalTemperature(parseFloat(e.target.value))
                    }
                    disabled={isStreaming}
                    className="px-2 py-1 border rounded text-sm w-20 bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600"
                  />
                </div>
                <div className="flex flex-col gap-1">
                  <label className="text-xs text-gray-600 dark:text-gray-400">
                    Top P
                  </label>
                  <input
                    type="number"
                    min="0"
                    max="1"
                    step="0.1"
                    value={localTopP !== null ? localTopP : settings.topP ?? ""}
                    onChange={(e) => setLocalTopP(parseFloat(e.target.value))}
                    disabled={isStreaming}
                    className="px-2 py-1 border rounded text-sm w-20 bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600"
                  />
                </div>
                <div className="flex flex-col gap-1">
                  <label className="text-xs text-gray-600 dark:text-gray-400">
                    Max Tokens
                  </label>
                  <input
                    type="number"
                    min="1"
                    max="8192"
                    step="1"
                    value={
                      localMaxTokens !== null
                        ? localMaxTokens
                        : settings.maxTokens ?? ""
                    }
                    onChange={(e) =>
                      setLocalMaxTokens(parseInt(e.target.value))
                    }
                    disabled={isStreaming}
                    className="px-2 py-1 border rounded text-sm w-20 bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600"
                  />
                </div>
                <Button
                  onClick={() => {
                    setLocalTemperature(null);
                    setLocalTopP(null);
                    setLocalMaxTokens(null);
                  }}
                  disabled={isStreaming}
                  size="sm"
                  variant="outline"
                  className="self-end text-xs"
                >
                  Reset to Global
                </Button>
              </div>
            )}

            {/* Attachment Panel */}
            {showAttachmentPanel && (
              <div className="flex flex-col gap-3 pb-3 border-b border-gray-200 dark:border-gray-700">
                {!showFileSelector ? (
                  // Two button interface
                  <div className="flex items-center gap-2">
                    <Button
                      onClick={handleFileUpload}
                      disabled={isUploadingFile}
                      size="sm"
                      variant="outline"
                      className="flex-1"
                    >
                      {isUploadingFile ? (
                        <Loader2 className="w-4 h-4 animate-spin" />
                      ) : (
                        "Upload New"
                      )}
                    </Button>
                    <Button
                      onClick={() => {
                        setShowFileSelector(true);
                        loadAvailableFiles();
                      }}
                      size="sm"
                      variant="outline"
                      className="flex-1"
                    >
                      Select from RAG
                    </Button>
                    <Button
                      onClick={() => {
                        setShowAttachmentPanel(false);
                        setShowFileSelector(false);
                      }}
                      size="sm"
                      variant="ghost"
                    >
                      <X className="w-4 h-4" />
                    </Button>
                  </div>
                ) : (
                  // File selector interface
                  <>
                    <div className="flex items-center justify-between">
                      <div className="text-sm font-medium">
                        Select Documents from RAG
                      </div>
                      <Button
                        onClick={() => {
                          setShowFileSelector(false);
                          setFileSearchQuery("");
                        }}
                        size="sm"
                        variant="ghost"
                      >
                        <X className="w-4 h-4" />
                      </Button>
                    </div>

                    {/* Search */}
                    <Input
                      value={fileSearchQuery}
                      onChange={(e) => setFileSearchQuery(e.target.value)}
                      placeholder="Search documents..."
                      className="text-sm"
                    />

                    {/* File List */}
                    <div className="border border-gray-200 dark:border-gray-700 rounded-lg max-h-64 overflow-y-auto">
                      {isLoadingFiles ? (
                        <div className="flex items-center justify-center p-4">
                          <Loader2 className="w-5 h-5 animate-spin text-gray-400" />
                        </div>
                      ) : availableFiles.filter((f) =>
                          f.file_name
                            .toLowerCase()
                            .includes(fileSearchQuery.toLowerCase())
                        ).length === 0 ? (
                        <div className="p-4 text-center text-sm text-gray-500 dark:text-gray-400">
                          {fileSearchQuery
                            ? "No documents match your search"
                            : "No documents available. Upload some documents first."}
                        </div>
                      ) : (
                        <div className="divide-y divide-gray-200 dark:divide-gray-700">
                          {availableFiles
                            .filter((f) =>
                              f.file_name
                                .toLowerCase()
                                .includes(fileSearchQuery.toLowerCase())
                            )
                            .map((file) => {
                              const isAttached = attachments.some(
                                (a) => a.file_path === file.file_path
                              );
                              return (
                                <div
                                  key={file.file_path}
                                  className={`p-2 hover:bg-gray-50 dark:hover:bg-gray-800 cursor-pointer transition-colors text-sm ${
                                    isAttached
                                      ? "bg-primary/5 dark:bg-primary/10"
                                      : ""
                                  }`}
                                  onClick={() =>
                                    toggleAttachment(file.file_path)
                                  }
                                >
                                  <div className="flex items-center gap-2">
                                    <input
                                      type="checkbox"
                                      checked={isAttached}
                                      onChange={() =>
                                        toggleAttachment(file.file_path)
                                      }
                                      className="w-3.5 h-3.5 rounded border-gray-300 text-primary focus:ring-primary"
                                      onClick={(e) => e.stopPropagation()}
                                    />
                                    <Paperclip className="w-4 h-4 text-gray-400 flex-shrink-0" />
                                    <div className="flex-1 min-w-0">
                                      <div className="font-medium truncate">
                                        {file.file_name}
                                      </div>
                                      <div className="text-xs text-gray-500 dark:text-gray-400">
                                        {file.file_type.toUpperCase()} •{" "}
                                        {file.chunk_count} chunks
                                      </div>
                                    </div>
                                  </div>
                                </div>
                              );
                            })}
                        </div>
                      )}
                    </div>

                    {/* Attached count */}
                    <div className="text-xs text-gray-600 dark:text-gray-400">
                      {attachments.length > 0
                        ? `${attachments.length} document${
                            attachments.length !== 1 ? "s" : ""
                          } attached`
                        : "No documents attached"}
                    </div>
                  </>
                )}
              </div>
            )}

            {/* Warning for images without vision model */}
            {attachments.some((a) => a.is_image) &&
              !loadedModelsByType["image-to-text"] && (
                <div className="flex items-center gap-2 px-3 py-2 bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-lg text-sm text-yellow-800 dark:text-yellow-200">
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className="w-4 h-4 flex-shrink-0"
                  >
                    <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"></path>
                    <line x1="12" y1="9" x2="12" y2="13"></line>
                    <line x1="12" y1="17" x2="12.01" y2="17"></line>
                  </svg>
                  <span>
                    Image attached. Please select a Vision (Image-to-Text) model
                    above to process images.
                  </span>
                </div>
              )}

            {/* Attached Documents and Images */}
            {attachments.length > 0 && (
              <div className="flex flex-wrap gap-2 pb-3 border-b border-gray-200 dark:border-gray-700">
                {attachments.map((attachment) => (
                  <div
                    key={attachment.file_path}
                    className="flex items-center gap-2 px-3 py-1.5 bg-primary/10 dark:bg-primary/20 rounded-lg text-sm"
                  >
                    {attachment.is_image ? (
                      <>
                        <img
                          src={convertFileSrc(attachment.file_path)}
                          alt={attachment.file_name}
                          className="w-8 h-8 object-cover rounded"
                        />
                        <span className="max-w-[200px] truncate">
                          {attachment.file_name}
                        </span>
                      </>
                    ) : (
                      <>
                        <Paperclip className="w-3.5 h-3.5" />
                        <span className="max-w-[200px] truncate">
                          {attachment.file_name}
                        </span>
                      </>
                    )}
                    <button
                      onClick={() =>
                        setAttachments((prev) =>
                          prev.filter(
                            (a) => a.file_path !== attachment.file_path
                          )
                        )
                      }
                      disabled={isStreaming}
                      className="hover:text-red-600 dark:hover:text-red-400 disabled:opacity-50"
                    >
                      <X className="w-3.5 h-3.5" />
                    </button>
                  </div>
                ))}
              </div>
            )}

            <div className="flex gap-3">
              <div className="flex items-end">
                <Button
                  onClick={() => {
                    setShowAttachmentPanel(!showAttachmentPanel);
                    setShowFileSelector(false);
                    setFileSearchQuery("");
                  }}
                  disabled={isStreaming}
                  size="icon"
                  variant="outline"
                  title="Attach Documents"
                >
                  <Paperclip className="w-5 h-5" />
                </Button>
              </div>
              <textarea
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={handleKeyPress}
                placeholder={
                  loadedModelsByType.text
                    ? "Type your message... (Shift+Enter for new line)"
                    : "Select a Text Generation model to start chatting"
                }
                disabled={isStreaming || !loadedModelsByType.text}
                className="flex-1 resize-none bg-transparent border-none outline-none min-h-[44px] max-h-[200px] text-gray-900 dark:text-white placeholder:text-gray-400 disabled:opacity-50 disabled:cursor-not-allowed flex items-center py-2"
                rows={1}
                style={{
                  height: "auto",
                  overflow: "auto",
                }}
                onInput={(e) => {
                  const target = e.target as HTMLTextAreaElement;
                  target.style.height = "auto";
                  target.style.height = target.scrollHeight + "px";
                }}
              />
              <div className="flex gap-2 self-end">
                <Button
                  onClick={() => setShowConfigPanel(!showConfigPanel)}
                  disabled={isStreaming}
                  size="icon"
                  variant="outline"
                  title="Toggle LLM Configuration"
                >
                  <svg
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    className="w-5 h-5"
                  >
                    <circle cx="12" cy="12" r="3"></circle>
                    <path d="M12 1v6m0 6v6m-6-6H0m12 0h6"></path>
                  </svg>
                </Button>
                {isStreaming ? (
                  <Button
                    onClick={handleStop}
                    variant="destructive"
                    size="icon"
                    title="Stop generating"
                  >
                    <StopCircle className="w-5 h-5" />
                  </Button>
                ) : (
                  <Button
                    onClick={handleSend}
                    disabled={
                      !input.trim() ||
                      !loadedModelsByType.text ||
                      (attachments.some((a) => a.is_image) &&
                        !loadedModelsByType["image-to-text"])
                    }
                    size="icon"
                  >
                    <Send className="w-5 h-5" />
                  </Button>
                )}
              </div>
            </div>
          </div>
        </Card>
      </div>
    </PageContainer>
  );
};
