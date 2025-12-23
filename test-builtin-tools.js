// Simple test to verify built-in tools are working
// You can run this in your browser console after the app starts

async function testBuiltinTools() {
  const { invoke } = window.__TAURI__.core;

  console.log("🧪 Testing Built-in MCP Tools...\n");

  try {
    // Test 1: Get all tools
    console.log("1️⃣ Getting all built-in tools...");
    const tools = await invoke("get_builtin_tools");
    console.log(
      `✅ Found ${tools.length} tools:`,
      tools.map((t) => t.name)
    );
    console.log("");

    // Test 2: Get system info
    console.log("2️⃣ Testing get_system_info...");
    const sysInfoResult = await invoke("execute_builtin_tool", {
      toolName: "get_system_info",
      arguments: {},
    });
    const sysInfo = JSON.parse(sysInfoResult.content[0].text);
    console.log("✅ System Info:", sysInfo);
    console.log("");

    // Test 3: Get current time (ISO8601)
    console.log("3️⃣ Testing get_current_time (ISO8601)...");
    const timeResult = await invoke("execute_builtin_tool", {
      toolName: "get_current_time",
      arguments: { format: "iso8601" },
    });
    const timeData = JSON.parse(timeResult.content[0].text);
    console.log("✅ Current Time:", timeData);
    console.log("");

    // Test 4: Get current time (readable)
    console.log("4️⃣ Testing get_current_time (readable)...");
    const readableTimeResult = await invoke("execute_builtin_tool", {
      toolName: "get_current_time",
      arguments: { format: "readable" },
    });
    const readableTime = JSON.parse(readableTimeResult.content[0].text);
    console.log("✅ Readable Time:", readableTime);
    console.log("");

    // Test 5: List directory (user's home directory)
    console.log("5️⃣ Testing list_directory...");
    const homeDir = await invoke("get_home_dir");
    const dirResult = await invoke("execute_builtin_tool", {
      toolName: "list_directory",
      arguments: {
        path: homeDir,
        recursive: false,
      },
    });
    const dirData = JSON.parse(dirResult.content[0].text);
    console.log(`✅ Listed ${dirData.entry_count} entries in ${dirData.path}`);
    console.log("First 5 entries:", dirData.entries.slice(0, 5));
    console.log("");

    // Test 6: Get all available tools (including external)
    console.log("6️⃣ Testing get_all_available_tools...");
    const allTools = await invoke("get_all_available_tools");
    console.log("✅ All Tools:", {
      builtinCount: allTools.builtin_tools.length,
      externalServers: Object.keys(allTools.external_servers).length,
    });
    console.log("");

    console.log("🎉 All tests passed!");
    console.log("\nBuilt-in tools are ready to use!");
    console.log("Try calling them from your chat interface.");
  } catch (error) {
    console.error("❌ Test failed:", error);
  }
}

// Run the test
testBuiltinTools();
