import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PageContainer } from "../layout";
import {
  Card,
  Button,
  Input,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "../ui";
import {
  Plus,
  Server,
  Trash2,
  RefreshCw,
  Play,
  Square,
  Wrench,
  Loader2,
} from "lucide-react";

interface McpServerConfig {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
  auto_connect?: boolean;
}

interface ToolInfo {
  name: string;
  description?: string;
}

interface BuiltinTool {
  name: string;
  description: string;
  input_schema: any;
  hidden_from_task_creation?: boolean;
}

interface McpServerInfo {
  name: string;
  config: McpServerConfig;
  status: string;
  tools: string[];
}

export const McpPage = () => {
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isAddDialogOpen, setIsAddDialogOpen] = useState(false);
  const [newServerName, setNewServerName] = useState("");
  const [newServerType, setNewServerType] = useState<"stdio" | "sse" | "http">(
    "stdio"
  );
  const [newServerCommand, setNewServerCommand] = useState("");
  const [newServerArgs, setNewServerArgs] = useState("");
  const [newServerUrl, setNewServerUrl] = useState("");
  const [envVars, setEnvVars] = useState<Array<{ key: string; value: string }>>(
    []
  );
  const [toolDetails, setToolDetails] = useState<ToolInfo[]>([]);
  const [loadingTools, setLoadingTools] = useState(false);
  const [connectingServer, setConnectingServer] = useState<string | null>(null);
  const [isBuiltinToolsOpen, setIsBuiltinToolsOpen] = useState(false);
  const [builtinTools, setBuiltinTools] = useState<BuiltinTool[]>([]);
  const [loadingBuiltinTools, setLoadingBuiltinTools] = useState(false);
  const [isServerDetailsOpen, setIsServerDetailsOpen] = useState(false);
  const [detailsServer, setDetailsServer] = useState<McpServerInfo | null>(
    null
  );

  useEffect(() => {
    loadServers();
  }, []);

  useEffect(() => {
    // Fetch tool details when a connected server details dialog is opened
    if (detailsServer && detailsServer.status === "connected") {
      fetchToolDetails(detailsServer.name);
    } else {
      setToolDetails([]);
    }
  }, [detailsServer]);

  const loadServers = async () => {
    setIsLoading(true);
    try {
      const result = await invoke<McpServerInfo[]>("get_mcp_servers");
      console.warn(result);
      setServers(result);
    } catch (error) {
      console.error("Failed to load MCP servers:", error);
    } finally {
      setIsLoading(false);
    }
  };

  const fetchToolDetails = async (serverName: string) => {
    setLoadingTools(true);
    try {
      const tools = await invoke<ToolInfo[]>("fetch_mcp_server_tools_details", {
        serverName,
      });
      setToolDetails(tools);
    } catch (error) {
      console.error("Failed to fetch tool details:", error);
      setToolDetails([]);
    } finally {
      setLoadingTools(false);
    }
  };

  const fetchBuiltinTools = async () => {
    setLoadingBuiltinTools(true);
    try {
      const tools = await invoke<BuiltinTool[]>("get_builtin_tools");
      setBuiltinTools(tools);
      setIsBuiltinToolsOpen(true);
    } catch (error) {
      console.error("Failed to fetch builtin tools:", error);
      setBuiltinTools([]);
    } finally {
      setLoadingBuiltinTools(false);
    }
  };

  // auto-connect helper removed (unused) — MCP auto-connect can be triggered via settings or explicit action

  const handleToggleAutoConnect = async (
    serverName: string,
    enabled: boolean
  ) => {
    try {
      await invoke("toggle_mcp_server_auto_connect", {
        serverName,
        autoConnect: enabled,
      });
      await loadServers();
    } catch (error) {
      console.error("Failed to toggle auto-connect:", error);
    }
  };

  const handleEnableAllAutoConnect = async () => {
    try {
      await invoke("enable_all_auto_connect");
      await loadServers();
    } catch (error) {
      console.error("Failed to enable all auto-connect:", error);
    }
  };

  const handleAddServer = async () => {
    // Validation based on server type
    if (!newServerName.trim()) {
      console.error("Server name is required");
      return;
    }

    if (newServerType === "stdio" && !newServerCommand.trim()) {
      console.error("Command is required for stdio server");
      return;
    }

    if (
      (newServerType === "sse" || newServerType === "http") &&
      !newServerUrl.trim()
    ) {
      console.error("URL is required for this server type");
      return;
    }

    try {
      const args = newServerArgs.trim()
        ? newServerArgs.split(" ").filter((a) => a.trim())
        : [];

      // Build env object from envVars array
      const env = envVars.reduce((acc, { key, value }) => {
        if (key.trim() && value.trim()) {
          acc[key.trim()] = value.trim();
        }
        return acc;
      }, {} as Record<string, string>);

      await invoke("add_mcp_server", {
        request: {
          name: newServerName.trim(),
          command:
            newServerType === "stdio" ? newServerCommand.trim() : undefined,
          args: newServerType === "stdio" && args.length > 0 ? args : undefined,
          env: Object.keys(env).length > 0 ? env : undefined,
          url: newServerType !== "stdio" ? newServerUrl.trim() : undefined,
          auto_connect: false,
        },
      });

      // Reset form
      setNewServerName("");
      setNewServerType("stdio");
      setNewServerCommand("");
      setNewServerArgs("");
      setNewServerUrl("");
      setEnvVars([]);
      setIsAddDialogOpen(false);
      await loadServers();
    } catch (error) {
      console.error("Failed to add server:", error);
    }
  };

  const handleRemoveServer = async (name: string) => {
    try {
      await invoke("remove_mcp_server", { serverName: name });
      await loadServers();
      if (detailsServer?.name === name) {
        setIsServerDetailsOpen(false);
        setDetailsServer(null);
      }
    } catch (error) {
      console.error("Failed to remove server:", error);
    }
  };

  const handleConnectServer = async (name: string) => {
    setConnectingServer(name);
    try {
      await invoke("connect_mcp_server", { serverName: name });
      await loadServers();
    } catch (error) {
      console.error("Failed to connect to server:", error);
    } finally {
      setConnectingServer(null);
    }
  };

  const handleDisconnectServer = async (name: string) => {
    try {
      await invoke("disconnect_mcp_server", { serverName: name });
      await loadServers();
    } catch (error) {
      console.error("Failed to disconnect from server:", error);
    }
  };

  const getStatusColor = (status: string) => {
    switch (status) {
      case "connected":
        return "text-green-600 bg-green-100 dark:bg-green-900/20";
      case "disconnected":
        return "text-gray-600 bg-gray-100 dark:bg-gray-800";
      case "error":
        return "text-red-600 bg-red-100 dark:bg-red-900/20";
      default:
        return "text-gray-600 bg-gray-100 dark:bg-gray-800";
    }
  };

  return (
    <PageContainer
      title="Model Context Protocol"
      description="Configure MCP servers and tools"
    >
      <div className="flex flex-col gap-4 h-full">
        {/* Servers List */}
        <div className="flex flex-col gap-4">
          {/* Header */}
          <Card className="p-4">
            <div className="flex items-center justify-between">
              <div>
                <h2 className="text-lg font-semibold mb-1">MCP Servers</h2>
                <p className="text-sm text-gray-600 dark:text-gray-400">
                  {servers.length} {servers.length === 1 ? "server" : "servers"}{" "}
                  configured
                </p>
              </div>
              <div className="flex items-center gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={fetchBuiltinTools}
                  disabled={loadingBuiltinTools}
                >
                  <Wrench className="w-4 h-4 mr-2" />
                  {loadingBuiltinTools ? "Loading..." : "Builtin Tools"}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleEnableAllAutoConnect}
                  disabled={servers.length === 0}
                >
                  Enable All Auto-Connect
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={loadServers}
                  disabled={isLoading}
                >
                  <RefreshCw
                    className={`w-5 h-5 ${isLoading ? "animate-spin" : ""}`}
                  />
                </Button>
                <Button onClick={() => setIsAddDialogOpen(true)}>
                  <Plus className="w-5 h-5 mr-2" />
                  Add Server
                </Button>
              </div>
            </div>
          </Card>

          {/* Servers List */}
          <div className="flex-1 overflow-y-auto space-y-3 px-2 py-1">
            {isLoading && servers.length === 0 ? (
              <Card className="p-8 text-center">
                <Loader2 className="w-12 h-12 mx-auto mb-3 text-primary animate-spin" />
                <p className="text-gray-600 dark:text-gray-400">
                  Loading servers...
                </p>
              </Card>
            ) : servers.length === 0 ? (
              <Card className="p-8 text-center">
                <Server className="w-12 h-12 mx-auto mb-3 text-gray-400" />
                <h3 className="text-lg font-semibold mb-2">
                  No MCP servers configured
                </h3>
                <p className="text-gray-600 dark:text-gray-400 mb-4">
                  Add your first MCP server to enable external tools and
                  integrations
                </p>
                <Button onClick={() => setIsAddDialogOpen(true)}>
                  <Plus className="w-5 h-5 mr-2" />
                  Add Your First Server
                </Button>
              </Card>
            ) : (
              servers.map((server) => (
                <Card
                  key={server.name}
                  className="p-3 transition-all hover:shadow-md"
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex items-start gap-2 flex-1 min-w-0">
                      <div className="flex-shrink-0 w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                        <Server className="w-4 h-4 text-primary" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <h3 className="text-sm font-semibold truncate">
                          {server.name}
                        </h3>
                        <p className="text-xs text-gray-600 dark:text-gray-400 truncate">
                          {server.config.command
                            ? `${server.config.command}${
                                server.config.args
                                  ? " " + server.config.args.join(" ")
                                  : ""
                              }`
                            : server.config.url || "No command"}
                        </p>
                        <div className="flex items-center gap-2 mt-1">
                          <span
                            className={`text-xs px-2 py-1 rounded-full ${getStatusColor(
                              server.status
                            )}`}
                          >
                            {server.status}
                          </span>
                          {server.tools.length > 0 && (
                            <span className="text-xs text-gray-600 dark:text-gray-400">
                              {server.tools.length}{" "}
                              {server.tools.length === 1 ? "tool" : "tools"}
                            </span>
                          )}
                          <label
                            className="flex items-center gap-1 cursor-pointer"
                            onClick={(e) => e.stopPropagation()}
                          >
                            <input
                              type="checkbox"
                              checked={server.config.auto_connect || false}
                              onChange={(e) => {
                                e.stopPropagation();
                                handleToggleAutoConnect(
                                  server.name,
                                  e.target.checked
                                );
                              }}
                              className="w-3 h-3 rounded border-gray-300"
                            />
                            <span className="text-xs text-gray-600 dark:text-gray-400">
                              Auto
                            </span>
                          </label>
                        </div>
                      </div>
                    </div>

                    <div className="flex flex-col gap-1">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          setDetailsServer(server);
                          setIsServerDetailsOpen(true);
                        }}
                      >
                        Details
                      </Button>
                      {server.status === "connected" ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => handleDisconnectServer(server.name)}
                          disabled={connectingServer !== null}
                        >
                          <Square className="w-3 h-3 mr-1" />
                          Disconnect
                        </Button>
                      ) : (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={() => handleConnectServer(server.name)}
                          disabled={connectingServer !== null}
                        >
                          {connectingServer === server.name ? (
                            <>
                              <Loader2 className="w-3 h-3 mr-1 animate-spin" />
                              Connecting...
                            </>
                          ) : (
                            <>
                              <Play className="w-3 h-3 mr-1" />
                              Connect
                            </>
                          )}
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleRemoveServer(server.name)}
                        disabled={connectingServer !== null}
                      >
                        <Trash2 className="w-3 h-3 mr-1" />
                        Remove
                      </Button>
                    </div>
                  </div>
                </Card>
              ))
            )}
          </div>
        </div>
      </div>

      {/* Add Server Dialog */}
      <Dialog open={isAddDialogOpen} onOpenChange={setIsAddDialogOpen}>
        <div className="space-y-4">
          <h2 className="text-xl font-bold">Add MCP Server</h2>
          <div className="space-y-3">
            <div>
              <label className="block text-sm font-medium mb-1">
                Server Name
              </label>
              <Input
                value={newServerName}
                onChange={(e) => setNewServerName(e.target.value)}
                placeholder="e.g., time-server"
              />
            </div>

            {/* Server Type Selection */}
            <div>
              <label className="block text-sm font-medium mb-1">
                Server Type
              </label>
              <div className="grid grid-cols-3 gap-2">
                <Button
                  variant={newServerType === "stdio" ? "primary" : "outline"}
                  onClick={() => setNewServerType("stdio")}
                  className="w-full"
                >
                  STDIO
                </Button>
                <Button
                  variant={newServerType === "sse" ? "primary" : "outline"}
                  onClick={() => setNewServerType("sse")}
                  className="w-full"
                >
                  SSE
                </Button>
                <Button
                  variant={newServerType === "http" ? "primary" : "outline"}
                  onClick={() => setNewServerType("http")}
                  className="w-full"
                >
                  HTTP
                </Button>
              </div>
            </div>

            {/* STDIO fields */}
            {newServerType === "stdio" && (
              <>
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Command
                  </label>
                  <Input
                    value={newServerCommand}
                    onChange={(e) => setNewServerCommand(e.target.value)}
                    placeholder="e.g., uvx"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium mb-1">
                    Arguments (optional, space-separated)
                  </label>
                  <Input
                    value={newServerArgs}
                    onChange={(e) => setNewServerArgs(e.target.value)}
                    placeholder="e.g., mcp-server-time --debug"
                  />
                </div>
              </>
            )}

            {/* SSE/HTTP fields */}
            {(newServerType === "sse" || newServerType === "http") && (
              <div>
                <label className="block text-sm font-medium mb-1">URL</label>
                <Input
                  value={newServerUrl}
                  onChange={(e) => setNewServerUrl(e.target.value)}
                  placeholder={
                    newServerType === "sse"
                      ? "e.g., http://localhost:9999/sse"
                      : "e.g., http://localhost:9999/mcp"
                  }
                />
                <p className="text-xs text-gray-500 mt-1">
                  {newServerType === "sse"
                    ? "URL must end with /sse"
                    : "URL must end with /mcp"}
                </p>
              </div>
            )}

            {/* Environment Variables */}
            <div>
              <label className="block text-sm font-medium mb-1">
                Environment Variables (optional)
              </label>
              <div className="space-y-2">
                {envVars.map((envVar, index) => (
                  <div key={index} className="flex gap-2">
                    <Input
                      value={envVar.key}
                      onChange={(e) => {
                        const newEnvVars = [...envVars];
                        newEnvVars[index].key = e.target.value;
                        setEnvVars(newEnvVars);
                      }}
                      placeholder="KEY"
                      className="flex-1"
                    />
                    <Input
                      value={envVar.value}
                      onChange={(e) => {
                        const newEnvVars = [...envVars];
                        newEnvVars[index].value = e.target.value;
                        setEnvVars(newEnvVars);
                      }}
                      placeholder="value"
                      className="flex-1"
                    />
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setEnvVars(envVars.filter((_, i) => i !== index));
                      }}
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  </div>
                ))}
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    setEnvVars([...envVars, { key: "", value: "" }])
                  }
                  className="w-full"
                >
                  <Plus className="w-4 h-4 mr-2" />
                  Add Environment Variable
                </Button>
              </div>
            </div>
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setIsAddDialogOpen(false)}>
              Cancel
            </Button>
            <Button onClick={handleAddServer}>Add Server</Button>
          </div>
        </div>
      </Dialog>

      {/* Server Details Dialog */}
      <Dialog open={isServerDetailsOpen} onOpenChange={setIsServerDetailsOpen}>
        <DialogContent
          className="max-w-3xl max-h-[80vh] flex flex-col"
          onClose={() => setIsServerDetailsOpen(false)}
        >
          <DialogHeader>
            <DialogTitle>{detailsServer?.name || "Server Details"}</DialogTitle>
          </DialogHeader>

          {detailsServer && (
            <div className="flex-1 overflow-y-auto mt-4 pr-2">
              <div className="space-y-4">
                <div>
                  <span
                    className={`text-sm px-3 py-1 rounded-full ${getStatusColor(
                      detailsServer.status
                    )}`}
                  >
                    {detailsServer.status}
                  </span>
                </div>

                <div>
                  <h3 className="text-sm font-semibold mb-2">Configuration</h3>
                  <div className="space-y-2 text-sm">
                    {detailsServer.config.command && (
                      <div>
                        <span className="text-gray-600 dark:text-gray-400">
                          Command:
                        </span>
                        <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                          {detailsServer.config.command}
                        </code>
                      </div>
                    )}
                    {detailsServer.config.args &&
                      detailsServer.config.args.length > 0 && (
                        <div>
                          <span className="text-gray-600 dark:text-gray-400">
                            Arguments:
                          </span>
                          <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                            {detailsServer.config.args.join(" ")}
                          </code>
                        </div>
                      )}
                    {detailsServer.config.url && (
                      <div>
                        <span className="text-gray-600 dark:text-gray-400">
                          URL:
                        </span>
                        <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                          {detailsServer.config.url}
                        </code>
                      </div>
                    )}
                    {detailsServer.config.env &&
                      Object.keys(detailsServer.config.env).length > 0 && (
                        <div>
                          <span className="text-gray-600 dark:text-gray-400">
                            Environment Variables:
                          </span>
                          <div className="ml-2 mt-1 space-y-1">
                            {Object.entries(detailsServer.config.env).map(
                              ([key, value]) => (
                                <code
                                  key={key}
                                  className="block px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded text-xs"
                                >
                                  {key}={value}
                                </code>
                              )
                            )}
                          </div>
                        </div>
                      )}
                  </div>
                </div>

                {detailsServer.status === "connected" && (
                  <div>
                    <h3 className="text-sm font-semibold mb-2 flex items-center gap-2">
                      <Wrench className="w-4 h-4" />
                      Available Tools ({toolDetails.length})
                    </h3>
                    {loadingTools ? (
                      <div className="flex items-center justify-center py-4">
                        <Loader2 className="w-5 h-5 animate-spin text-primary" />
                      </div>
                    ) : toolDetails.length > 0 ? (
                      <div className="space-y-2">
                        {toolDetails.map((tool) => (
                          <div
                            key={tool.name}
                            className="p-3 bg-gray-100 dark:bg-gray-800 rounded"
                          >
                            <div className="font-semibold text-sm">
                              {tool.name}
                            </div>
                            {tool.description && (
                              <div className="text-xs text-gray-600 dark:text-gray-400 mt-1">
                                {tool.description}
                              </div>
                            )}
                          </div>
                        ))}
                      </div>
                    ) : (
                      <p className="text-sm text-gray-600 dark:text-gray-400">
                        No tools available
                      </p>
                    )}
                  </div>
                )}
              </div>
            </div>
          )}

          <div className="flex justify-end mt-4 pt-4 border-t border-gray-200 dark:border-gray-800 flex-shrink-0">
            <Button onClick={() => setIsServerDetailsOpen(false)}>Close</Button>
          </div>
        </DialogContent>
      </Dialog>

      {/* Builtin Tools Dialog */}
      <Dialog open={isBuiltinToolsOpen} onOpenChange={setIsBuiltinToolsOpen}>
        <DialogContent
          className="max-w-3xl max-h-[80vh] flex flex-col"
          onClose={() => setIsBuiltinToolsOpen(false)}
        >
          <DialogHeader>
            <DialogTitle>Built-in Tools</DialogTitle>
          </DialogHeader>

          <div className="flex-1 overflow-y-auto mt-4 pr-2">
            {loadingBuiltinTools ? (
              <div className="flex items-center justify-center py-8">
                <Loader2 className="w-8 h-8 animate-spin text-primary" />
              </div>
            ) : builtinTools.length === 0 ? (
              <div className="text-center py-8">
                <Wrench className="w-12 h-12 mx-auto mb-3 text-gray-400" />
                <p className="text-gray-600 dark:text-gray-400">
                  No builtin tools available
                </p>
              </div>
            ) : (
              <div className="space-y-3">
                {builtinTools.map((tool, index) => (
                  <Card key={index} className="p-4">
                    <div className="flex items-start gap-3">
                      <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-primary/10 flex items-center justify-center">
                        <Wrench className="w-5 h-5 text-primary" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <h3 className="font-semibold text-sm mb-1">
                          {tool.name}
                        </h3>
                        <p className="text-sm text-gray-600 dark:text-gray-400 whitespace-pre-wrap">
                          {tool.description}
                        </p>
                      </div>
                    </div>
                  </Card>
                ))}
              </div>
            )}
          </div>

          <div className="flex justify-end mt-4 pt-4 border-t border-gray-200 dark:border-gray-800 flex-shrink-0">
            <Button onClick={() => setIsBuiltinToolsOpen(false)}>Close</Button>
          </div>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
};
