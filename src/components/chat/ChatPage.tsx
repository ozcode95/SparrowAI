import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { PageContainer } from "../layout";
import { Card, Button } from "../ui";
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
  } = useAppStore();
  const { settings } = useAppStore();
  const { loadedModel } = useAppStore();
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [currentStreamingMessage, setCurrentStreamingMessage] = useState("");
  const [toolCalls, setToolCalls] = useState<ToolCall[]>([]);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [currentChatMessages, currentStreamingMessage]);

  // Load messages for active session
  useEffect(() => {
    if (activeChatSessionId && !temporarySession) {
      invoke<ChatMessageType[]>("get_session_messages", {
        sessionId: activeChatSessionId,
      })
        .then((messages) => {
          setCurrentChatMessages(messages);
        })
        .catch((error) => {
          console.error("Failed to load session messages:", error);
        });
    }
  }, [activeChatSessionId, temporarySession, setCurrentChatMessages]);

  // Listen for streaming tokens
  useEffect(() => {
    const unlisten = listen<{ token: string; finished: boolean }>(
      "chat-token",
      (event) => {
        if (event.payload.finished) {
          setIsStreaming(false);
          setCurrentStreamingMessage("");
        } else {
          setCurrentStreamingMessage((prev) => prev + event.payload.token);
        }
      }
    );

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for tool calls
  useEffect(() => {
    const unlisten = listen<ToolCall>("tool-call", (event) => {
      setToolCalls((prev) => [...prev, event.payload]);
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Listen for chat completion
  useEffect(() => {
    const unlisten = listen<{ response: string; tokens_per_second?: number }>(
      "chat-complete",
      async (event) => {
        setIsStreaming(false);
        setCurrentStreamingMessage("");

        const assistantMessage: ChatMessageType = {
          id: crypto.randomUUID(),
          role: "assistant",
          content: event.payload.response,
          timestamp: Date.now(),
          toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
        };

        // Add to temporary session or active session
        if (temporarySession) {
          const result = await invoke<[any, ChatMessageType]>(
            "add_message_to_temporary_session",
            {
              session: temporarySession,
              role: "assistant",
              content: event.payload.response,
              tokensPerSecond: event.payload.tokens_per_second,
              isError: false,
            }
          );
          const [updatedSession] = result;
          setTemporarySession(updatedSession);
          addMessageToCurrentChat(assistantMessage);
        } else if (activeChatSessionId) {
          await invoke("add_message_to_session", {
            sessionId: activeChatSessionId,
            role: "assistant",
            content: event.payload.response,
            tokensPerSecond: event.payload.tokens_per_second,
            isError: false,
          });
          addMessageToCurrentChat(assistantMessage);
        }

        setToolCalls([]);
      }
    );

    return () => {
      unlisten.then((fn) => fn());
    };
  }, [
    temporarySession,
    activeChatSessionId,
    addMessageToCurrentChat,
    setTemporarySession,
    toolCalls,
  ]);

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
    if (!loadedModel) {
      alert("Please load a model first in Settings");
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

    // Add to temporary session or active session
    try {
      if (temporarySession) {
        const result = await invoke<[any, ChatMessageType]>(
          "add_message_to_temporary_session",
          {
            session: temporarySession,
            role: "user",
            content: input.trim(),
            tokensPerSecond: null,
            isError: false,
          }
        );
        const [updatedSession] = result;
        setTemporarySession(updatedSession);
      } else if (activeChatSessionId) {
        await invoke("add_message_to_session", {
          sessionId: activeChatSessionId,
          role: "user",
          content: input.trim(),
          tokensPerSecond: null,
          isError: false,
        });
      } else {
        // Create a new temporary session
        const newSession = await invoke<any>("create_chat_session", {
          title: null,
        });
        setTemporarySession(newSession);
        await invoke<[any, ChatMessageType]>(
          "add_message_to_temporary_session",
          {
            session: newSession,
            role: "user",
            content: input.trim(),
            tokensPerSecond: null,
            isError: false,
          }
        );
      }

      setInput("");
      setIsStreaming(true);
      setToolCalls([]);

      // Start streaming chat
      await invoke("stream_chat_completion", {
        sessionId: temporarySession?.id || activeChatSessionId,
        message: input.trim(),
        modelName: loadedModel,
        includeHistory: settings.includeConversationHistory,
        systemPrompt: settings.systemPrompt || null,
        temperature: settings.temperature,
        topP: settings.topP,
        seed: settings.seed,
        maxTokens: settings.maxTokens,
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

  return (
    <PageContainer title="Chat" description="Conversation with AI assistant">
      <div className="flex flex-col h-full">
        {/* Messages Area */}
        <div className="flex-1 overflow-y-auto mb-4 space-y-4 px-2">
          {currentChatMessages.length === 0 && !isStreaming && (
            <Card className="p-8 text-center">
              <Bot className="w-16 h-16 mx-auto mb-4 text-gray-400" />
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
              className={`flex gap-3 ${
                message.role === "user" ? "justify-end" : "justify-start"
              }`}
            >
              {message.role === "assistant" && (
                <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                  <Bot className="w-5 h-5 text-primary" />
                </div>
              )}

              <Card
                className={`max-w-[80%] p-4 ${
                  message.role === "user" ? "bg-primary text-white" : ""
                }`}
              >
                <div className="prose prose-sm max-w-none dark:prose-invert">
                  <ReactMarkdown>{message.content}</ReactMarkdown>
                </div>

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
            <div className="flex gap-3 justify-start">
              <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                <Bot className="w-5 h-5 text-primary" />
              </div>
              <Card className="max-w-[80%] p-4">
                <div className="prose prose-sm max-w-none dark:prose-invert">
                  <ReactMarkdown>{currentStreamingMessage}</ReactMarkdown>
                </div>
              </Card>
            </div>
          )}

          {/* Loading indicator */}
          {isStreaming && !currentStreamingMessage && (
            <div className="flex gap-3 justify-start">
              <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                <Bot className="w-5 h-5 text-primary animate-pulse" />
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
          <div className="flex gap-3">
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyPress}
              placeholder={
                loadedModel
                  ? "Type your message... (Shift+Enter for new line)"
                  : "Load a model to start chatting"
              }
              disabled={isStreaming || !loadedModel}
              className="flex-1 resize-none bg-transparent border-none outline-none min-h-[44px] max-h-[200px] text-gray-900 dark:text-white placeholder:text-gray-400 disabled:opacity-50"
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
            <Button
              onClick={handleSend}
              disabled={!input.trim() || isStreaming || !loadedModel}
              className="self-end"
              size="icon"
            >
              {isStreaming ? (
                <Loader2 className="w-5 h-5 animate-spin" />
              ) : (
                <Send className="w-5 h-5" />
              )}
            </Button>
          </div>
        </Card>
      </div>
    </PageContainer>
  );
};
