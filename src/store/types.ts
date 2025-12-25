// Type definitions for the entire store
export type ThemeMode = "light" | "dark";
export type ThemeColor = "orange" | "blue" | "green" | "purple" | "red";

export interface ThemeSlice {
  themeMode: ThemeMode;
  themeColor: ThemeColor;
  setThemeMode: (mode: ThemeMode) => void;
  setThemeColor: (color: ThemeColor) => void;
  toggleThemeMode: () => void;
}

export type NotificationType = "success" | "error" | "warning" | "info";
export type PageType =
  | "chat"
  | "models"
  | "documents"
  | "mcp"
  | "tasks"
  | "settings";

export interface Notification {
  message: string;
  type: NotificationType;
  timestamp: number;
  timeout?: number | null;
}

export interface UISlice {
  sidebarCollapsed: boolean;
  settingsDialogOpen: boolean;
  currentPage: PageType;
  notification: Notification | null;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleSidebar: () => void;
  setSettingsDialogOpen: (open: boolean) => void;
  setCurrentPage: (page: PageType) => void;
  showNotification: (
    message: string,
    type?: NotificationType,
    timeout?: number | null
  ) => void;
  clearNotification: () => void;
}

export interface SearchResult {
  id: string;
  [key: string]: any;
}

export interface ModelsSlice {
  searchQuery: string;
  searchResults: SearchResult[];
  selectedModel: any | null;
  isSearching: boolean;
  downloadingModels: Set<string>;
  downloadProgress: Record<string, { progress: number; currentFile: string }>;
  downloadedModels: Set<string>;
  isOvmsRunning: boolean;
  loadedModel: string | null;
  setSearchQuery: (query: string) => void;
  setSearchResults: (results: SearchResult[]) => void;
  setSelectedModel: (model: any) => void;
  setIsSearching: (isSearching: boolean) => void;
  clearSearch: () => void;
  setModelDownloading: (modelId: string, isDownloading: boolean) => void;
  isModelDownloading: (modelId: string) => boolean;
  hasAnyDownloading: () => boolean;
  isModelDownloaded: (modelId: string) => boolean;
  setDownloadProgress: (
    modelId: string,
    progress: number,
    currentFile?: string
  ) => void;
  getDownloadProgress: (modelId: string) => {
    progress: number;
    currentFile: string;
  };
  addDownloadedModel: (modelId: string) => void;
  removeDownloadedModel: (modelId: string) => void;
  setDownloadedModels: (modelIds: string[]) => void;
  setIsOvmsRunning: (isRunning: boolean) => void;
  setLoadedModel: (modelId: string | null) => void;
  getLoadedModel: () => Promise<string | null>;
}

export interface Settings {
  includeConversationHistory: boolean;
  systemPrompt: string;
  temperature: number;
  topP: number;
  seed: number | null;
  maxTokens: number | null;
  maxCompletionTokens: number | null;
  useRAG: boolean;
  enableAutostart: boolean;
  startMinimized: boolean;
}

export interface SettingsSlice {
  settings: Settings;
  updateSettings: (newSettings: Partial<Settings>) => void;
  resetSettings: () => void;
  updateSetting: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  timestamp: number;
  thinking?: string;
  toolCalls?: any[];
  attachments?: Array<{
    file_path: string;
    file_name: string;
    file_type: string;
  }>;
  [key: string]: any;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  created_at: number;
  updated_at: number;
  [key: string]: any;
}

export interface ChatSlice {
  chatSessions: Record<string, ChatSession>;
  activeChatSessionId: string | null;
  currentChatMessages: ChatMessage[];
  temporarySession: ChatSession | null;
  setChatSessions: (sessions: Record<string, ChatSession>) => void;
  setActiveChatSessionId: (sessionId: string | null) => void;
  setCurrentChatMessages: (messages: ChatMessage[]) => void;
  setTemporarySession: (session: ChatSession | null) => void;
  addChatSession: (session: ChatSession) => void;
  updateChatSession: (sessionId: string, updates: Partial<ChatSession>) => void;
  removeChatSession: (sessionId: string) => void;
  addMessageToCurrentChat: (message: ChatMessage) => void;
  clearCurrentChatMessages: () => void;
  clearTemporarySession: () => void;
  getActiveSession: () => ChatSession | null | undefined;
  getChatSessionsArray: () => ChatSession[];
  getRecentChatSessions: (limit?: number) => ChatSession[];
}

export interface AppState
  extends ThemeSlice,
    UISlice,
    ModelsSlice,
    SettingsSlice,
    ChatSlice {}
