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
  const [newServerCommand, setNewServerCommand] = useState("");
  const [newServerArgs, setNewServerArgs] = useState("");
  const [selectedServer, setSelectedServer] = useState<McpServerInfo | null>(
    null
  );

  useEffect(() => {
    loadServers();
  }, []);

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

  const handleAddServer = async () => {
    if (!newServerName.trim() || !newServerCommand.trim()) {
      alert("Please provide both a name and command for the server");
      return;
    }

    try {
      const args = newServerArgs.trim()
        ? newServerArgs.split(" ").filter((a) => a.trim())
        : [];

      await invoke("add_mcp_server", {
        request: {
          name: newServerName.trim(),
          command: newServerCommand.trim(),
          args: args.length > 0 ? args : undefined,
          env: undefined,
          url: undefined,
        },
      });

      setNewServerName("");
      setNewServerCommand("");
      setNewServerArgs("");
      setIsAddDialogOpen(false);
      await loadServers();
    } catch (error) {
      console.error("Failed to add server:", error);
      alert(`Failed to add server: ${error}`);
    }
  };

  const handleRemoveServer = async (name: string) => {
    if (!confirm(`Are you sure you want to remove the server "${name}"?`)) {
      return;
    }

    try {
      await invoke("remove_mcp_server", { name });
      await loadServers();
      if (selectedServer?.name === name) {
        setSelectedServer(null);
      }
    } catch (error) {
      console.error("Failed to remove server:", error);
      alert(`Failed to remove server: ${error}`);
    }
  };

  const handleConnectServer = async (name: string) => {
    try {
      await invoke("connect_mcp_server", { name });
      await loadServers();
    } catch (error) {
      console.error("Failed to connect to server:", error);
      alert(`Failed to connect: ${error}`);
    }
  };

  const handleDisconnectServer = async (name: string) => {
    try {
      await invoke("disconnect_mcp_server", { name });
      await loadServers();
    } catch (error) {
      console.error("Failed to disconnect from server:", error);
      alert(`Failed to disconnect: ${error}`);
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
          <div className="flex-1 overflow-y-auto space-y-3">
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
                  className={`p-4 cursor-pointer transition-all hover:shadow-md ${
                    selectedServer?.name === server.name
                      ? "ring-2 ring-primary"
                      : ""
                  }`}
                  onClick={() => setSelectedServer(server)}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="flex items-start gap-3 flex-1 min-w-0">
                      <div className="flex-shrink-0 w-10 h-10 rounded-full bg-primary/10 flex items-center justify-center">
                        <Server className="w-5 h-5 text-primary" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <h3 className="font-semibold truncate">
                          {server.name}
                        </h3>
                        <p className="text-sm text-gray-600 dark:text-gray-400 truncate">
                          {server.config.command ||
                            server.config.url ||
                            "No command"}
                        </p>
                        <div className="flex items-center gap-2 mt-2">
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
                        </div>
                      </div>
                    </div>

                    <div className="flex flex-col gap-2">
                      {server.status === "connected" ? (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleDisconnectServer(server.name);
                          }}
                        >
                          <Square className="w-4 h-4 mr-2" />
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
                        >
                          <Play className="w-4 h-4 mr-2" />
                          Connect
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleRemoveServer(server.name);
                        }}
                      >
                        <Trash2 className="w-4 h-4 mr-2" />
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

                {selectedServer.tools.length > 0 && (
                  <div>
                    <h3 className="text-sm font-semibold mb-2 flex items-center gap-2">
                      <Wrench className="w-4 h-4" />
                      Available Tools ({selectedServer.tools.length})
                    </h3>
                    <div className="space-y-2 max-h-64 overflow-y-auto">
                      {selectedServer.tools.map((tool) => (
                        <div
                          key={tool}
                          className="p-2 bg-gray-100 dark:bg-gray-800 rounded text-sm"
                        >
                          {tool}
                        </div>
                      ))}
                    </div>
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
            <div>
              <label className="block text-sm font-medium mb-1">Command</label>
              <Input
                value={newServerCommand}
                onChange={(e) => setNewServerCommand(e.target.value)}
                placeholder="e.g., uvx mcp-server-time"
              />
            </div>
            <div>
              <label className="block text-sm font-medium mb-1">
                Arguments (optional, space-separated)
              </label>
              <Input
                value={newServerArgs}
                onChange={(e) => setNewServerArgs(e.target.value)}
                placeholder="e.g., --debug --port 3000"
              />
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
