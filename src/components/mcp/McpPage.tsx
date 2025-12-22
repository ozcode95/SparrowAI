import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PageContainer } from "../layout";
import { Card, Button, Input, Dialog } from "../ui";
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
  const [selectedServer, setSelectedServer] = useState<McpServerInfo | null>(
    null
  );
  const [toolDetails, setToolDetails] = useState<ToolInfo[]>([]);
  const [loadingTools, setLoadingTools] = useState(false);
  const [connectingServer, setConnectingServer] = useState<string | null>(null);

  useEffect(() => {
    loadServers();
  }, []);

  useEffect(() => {
    // Fetch tool details when a connected server is selected
    if (selectedServer && selectedServer.status === "connected") {
      fetchToolDetails(selectedServer.name);
    } else {
      setToolDetails([]);
    }
  }, [selectedServer]);

  const loadServers = async () => {
    setIsLoading(true);
    try {
      const result = await invoke<McpServerInfo[]>("get_mcp_servers");
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

  const autoConnectServers = async () => {
    try {
      const connected = await invoke<string[]>("auto_connect_mcp_servers");
      if (connected.length > 0) {
        console.log("Auto-connected servers:", connected);
        await loadServers();
      }
    } catch (error) {
      console.error("Failed to auto-connect servers:", error);
    }
  };

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
      (newServerType === "sse" || newServerType === "websocket") &&
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
      if (selectedServer?.name === name) {
        setSelectedServer(null);
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
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6 h-full">
        {/* Servers List Column */}
        <div className="lg:col-span-2 flex flex-col gap-4">
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
                  className={`p-3 cursor-pointer transition-all hover:shadow-md ${
                    selectedServer?.name === server.name
                      ? "ring-1 ring-primary"
                      : ""
                  }`}
                  onClick={() => setSelectedServer(server)}
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
                      {server.status === "connected" ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDisconnectServer(server.name);
                          }}
                          disabled={connectingServer !== null}
                        >
                          <Square className="w-3 h-3 mr-1" />
                          Disconnect
                        </Button>
                      ) : (
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleConnectServer(server.name);
                          }}
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
                        onClick={(e) => {
                          e.stopPropagation();
                          handleRemoveServer(server.name);
                        }}
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

        {/* Details Column */}
        <div className="flex flex-col gap-4">
          <Card className="p-6 flex-1">
            {selectedServer ? (
              <div className="space-y-4">
                <div>
                  <h2 className="text-xl font-bold mb-2">
                    {selectedServer.name}
                  </h2>
                  <span
                    className={`text-sm px-3 py-1 rounded-full ${getStatusColor(
                      selectedServer.status
                    )}`}
                  >
                    {selectedServer.status}
                  </span>
                </div>

                <div>
                  <h3 className="text-sm font-semibold mb-2">Configuration</h3>
                  <div className="space-y-2 text-sm">
                    {selectedServer.config.command && (
                      <div>
                        <span className="text-gray-600 dark:text-gray-400">
                          Command:
                        </span>
                        <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                          {selectedServer.config.command}
                        </code>
                      </div>
                    )}
                    {selectedServer.config.args &&
                      selectedServer.config.args.length > 0 && (
                        <div>
                          <span className="text-gray-600 dark:text-gray-400">
                            Arguments:
                          </span>
                          <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                            {selectedServer.config.args.join(" ")}
                          </code>
                        </div>
                      )}
                    {selectedServer.config.url && (
                      <div>
                        <span className="text-gray-600 dark:text-gray-400">
                          URL:
                        </span>
                        <code className="ml-2 px-2 py-1 bg-gray-100 dark:bg-gray-800 rounded">
                          {selectedServer.config.url}
                        </code>
                      </div>
                    )}
                  </div>
                </div>

                {selectedServer.status === "connected" &&
                  toolDetails.length > 0 && (
                    <div>
                      <h3 className="text-sm font-semibold mb-2 flex items-center gap-2">
                        <Wrench className="w-4 h-4" />
                        Available Tools ({toolDetails.length})
                      </h3>
                      {loadingTools ? (
                        <div className="flex items-center justify-center py-4">
                          <Loader2 className="w-5 h-5 animate-spin text-primary" />
                        </div>
                      ) : (
                        <div className="space-y-2 max-h-64 overflow-y-auto">
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
                      )}
                    </div>
                  )}
              </div>
            ) : (
              <div className="flex flex-col items-center justify-center h-full text-center">
                <Server className="w-16 h-16 mb-4 text-gray-400" />
                <p className="text-gray-600 dark:text-gray-400">
                  Select a server to view details
                </p>
              </div>
            )}
          </Card>
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
    </PageContainer>
  );
};
