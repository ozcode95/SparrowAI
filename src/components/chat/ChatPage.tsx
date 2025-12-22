import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PageContainer } from "../layout";
import { Card, Button } from "../ui";
import { MessageContent } from "./MessageContent";
import { useAppStore } from "@/store";
import { Send, Bot, User, Loader2, Wrench } from "lucide-react";
import ReactMarkdown from "react-markdown";
import type { ChatMessage as ChatMessageType } from "@/store/types";

interface ToolCall {
  tool_name: string;
  arguments: string;
  result: string;
}

export const ChatPage = () => {
  const {
    activeChatSessionId,
    currentChatMessages,
    addMessageToCurrentChat,
    setCurrentChatMessages,
    temporarySession,
    setTemporarySession,
    setActiveChatSessionId,
  } = useAppStore();
  const { settings } = useAppStore();
  const { loadedModel, setLoadedModel, downloadedModels } = useAppStore();
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [currentStreamingMessage, setCurrentStreamingMessage] = useState("");
  const [toolCalls, setToolCalls] = useState<ToolCall[]>([]);
  const [isLoadingModel, setIsLoadingModel] = useState(false);

  // Local LLM configuration (optional overrides)
  const [localTemperature, setLocalTemperature] = useState<number | null>(null);
  const [localTopP, setLocalTopP] = useState<number | null>(null);
  const [localMaxTokens, setLocalMaxTokens] = useState<number | null>(null);
  const [showConfigPanel, setShowConfigPanel] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [currentChatMessages, currentStreamingMessage]);

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
          console.error("Failed to load session messages:", error);
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

    const unlisten = listen<{ token: string; finished: boolean }>(
      "chat-token",
      async (event) => {
        if (event.payload.finished) {
          console.log(
            "Chat streaming finished, accumulated message length:",
            accumulatedMessage.length
          );

          // Save the complete assistant message to chat session
          if (accumulatedMessage.trim()) {
            try {
              // Save to backend
              if (activeChatSessionId) {
                console.log(
                  "Saving assistant message to session:",
                  activeChatSessionId
                );
                await invoke("add_message_to_session", {
                  sessionId: activeChatSessionId,
                  role: "assistant",
                  content: accumulatedMessage,
                  tokensPerSecond: null,
                  isError: false,
                });
                console.log("Assistant message saved successfully");

                // Add to UI
                const assistantMessage: ChatMessageType = {
                  id: crypto.randomUUID(),
                  role: "assistant",
                  content: accumulatedMessage,
                  timestamp: Date.now(),
                  toolCalls:
                    accumulatedToolCalls.length > 0
                      ? accumulatedToolCalls
                      : undefined,
                };
                addMessageToCurrentChat(assistantMessage);
              } else {
                console.warn("No active session ID to save message");
              }
            } catch (error) {
              console.error("Failed to save assistant message:", error);
            }
          }

          // Clear streaming state
          setCurrentStreamingMessage("");
          setToolCalls([]);
          setIsStreaming(false);
          accumulatedMessage = "";
          accumulatedToolCalls = [];
        } else {
          // Accumulate the token
          accumulatedMessage += event.payload.token;
          setCurrentStreamingMessage((prev) => prev + event.payload.token);
        }
      }
    );

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

  // Listen for chat errors
  useEffect(() => {
    const unlisten = listen<{ error: string }>("chat-error", (event) => {
      setIsStreaming(false);
      setCurrentStreamingMessage("");
      console.error("Chat error:", event.payload.error);

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

    // Check if model is loaded
    if (!loadedModel) {
      alert("Please select a model first");
      return;
    }

    const userMessage: ChatMessageType = {
      id: crypto.randomUUID(),
      role: "user",
      content: input.trim(),
      timestamp: Date.now(),
    };

    // Add user message to UI immediately
    addMessageToCurrentChat(userMessage);

    const messageContent = input.trim();
    setInput("");
    setIsStreaming(true);
    setToolCalls([]);

    try {
      let sessionToUse = activeChatSessionId;

      console.log("handleSend - activeChatSessionId:", activeChatSessionId);
      console.log("handleSend - temporarySession:", temporarySession);

      // Create new session if this is the first message (no active session)
      if (!activeChatSessionId && !temporarySession) {
        console.log("Creating new chat session...");
        const newSession = await invoke<any>("create_chat_session", {
          title: null,
        });
        console.log("New session created:", newSession);
        sessionToUse = newSession.id;
        setActiveChatSessionId(newSession.id);

        // Add user message to the new session
        console.log("Adding user message to new session:", newSession.id);
        await invoke("add_message_to_session", {
          sessionId: newSession.id,
          role: "user",
          content: messageContent,
          tokensPerSecond: null,
          isError: false,
        });
        console.log("User message added to new session");

        // Trigger sidebar refresh
        window.dispatchEvent(new CustomEvent("chat-session-created"));
      } else if (activeChatSessionId) {
        // Verify the session exists in backend before adding message
        console.log("Checking if session exists:", activeChatSessionId);
        try {
          await invoke("get_session_messages", {
            sessionId: activeChatSessionId,
          });
          console.log("Session exists, adding message");

          // Add to existing active session
          await invoke("add_message_to_session", {
            sessionId: activeChatSessionId,
            role: "user",
            content: messageContent,
            tokensPerSecond: null,
            isError: false,
          });
          console.log("User message added to existing session");
        } catch (error) {
          console.error("Session not found in backend, creating new one");
          // Session doesn't exist in backend, create a new one
          const newSession = await invoke<any>("create_chat_session", {
            title: null,
          });
          console.log("New session created:", newSession);
          sessionToUse = newSession.id;
          setActiveChatSessionId(newSession.id);

          // Add user message to the new session
          await invoke("add_message_to_session", {
            sessionId: newSession.id,
            role: "user",
            content: messageContent,
            tokensPerSecond: null,
            isError: false,
          });
          console.log("User message added to new session");

          // Trigger sidebar refresh
          window.dispatchEvent(new CustomEvent("chat-session-created"));
        }
      }

      console.log("Using session for chat:", sessionToUse);

      // Prepare LLM configuration (use local overrides if set, otherwise global settings)
      const effectiveTemperature =
        localTemperature !== null ? localTemperature : settings.temperature;
      const effectiveTopP = localTopP !== null ? localTopP : settings.topP;
      const effectiveMaxTokens =
        localMaxTokens !== null ? localMaxTokens : settings.maxTokens;

      // Start streaming chat with all configuration
      // Strip 'OpenVINO/' prefix from model name if present
      const modelNameForChat = loadedModel.startsWith("OpenVINO/")
        ? loadedModel.substring("OpenVINO/".length)
        : loadedModel;

      await invoke("chat_with_loaded_model_streaming", {
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
      });
    } catch (error) {
      console.error("Failed to send message:", error);
      setIsStreaming(false);
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleLoadModel = async (modelId: string) => {
    setIsLoadingModel(true);
    try {
      // If a model is already loaded, unload it first
      if (loadedModel) {
        console.log(`Unloading current model: ${loadedModel}`);
        await invoke("unload_model");
        console.log("Model unloaded successfully");
      }

      // Load the new model
      await invoke("load_model", { modelId });
      setLoadedModel(modelId);
    } catch (error) {
      console.error("Failed to load model:", error);
    } finally {
      setIsLoadingModel(false);
    }
  };

  return (
    <PageContainer
      title="Chat"
      description="Conversation with AI assistant"
      actions={
        <div className="flex items-center gap-3 py-1 pr-2">
          <select
            value={loadedModel || ""}
            onChange={(e) => e.target.value && handleLoadModel(e.target.value)}
            disabled={isLoadingModel || isStreaming}
            className="px-3 py-1.5 border rounded-lg bg-white dark:bg-gray-800 border-gray-300 dark:border-gray-600 text-sm disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <option value="">Select a model...</option>
            {Array.from(downloadedModels).map((modelId) => (
              <option key={modelId} value={modelId}>
                {modelId}
              </option>
            ))}
          </select>
          {isLoadingModel && <Loader2 className="w-4 h-4 animate-spin" />}
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
                {loadedModel
                  ? `Using ${loadedModel}. Type a message below to begin.`
                  : "Load a model in Settings to start chatting."}
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
                  <MessageContent content={message.content} />
                ) : (
                  <div className="prose prose-sm max-w-none dark:prose-invert">
                    <ReactMarkdown>{message.content}</ReactMarkdown>
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
                        : settings.temperature
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
                    value={localTopP !== null ? localTopP : settings.topP}
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
                        : settings.maxTokens
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

            <div className="flex gap-3">
              <textarea
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={handleKeyPress}
                placeholder={
                  loadedModel
                    ? "Type your message... (Shift+Enter for new line)"
                    : "Select a model to start chatting"
                }
                disabled={isStreaming || !loadedModel}
                className="flex-1 resize-none bg-transparent border-none outline-none min-h-[44px] max-h-[200px] text-gray-900 dark:text-white placeholder:text-gray-400 disabled:opacity-50 disabled:cursor-not-allowed"
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
                <Button
                  onClick={handleSend}
                  disabled={!input.trim() || isStreaming || !loadedModel}
                  className=""
                  size="icon"
                >
                  {isStreaming ? (
                    <Loader2 className="w-5 h-5 animate-spin" />
                  ) : (
                    <Send className="w-5 h-5" />
                  )}
                </Button>
              </div>
            </div>
          </div>
        </Card>
      </div>
    </PageContainer>
  );
};
