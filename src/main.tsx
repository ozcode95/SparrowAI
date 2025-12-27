import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";
import { logInfo } from "./lib/logger";
import { ErrorBoundary } from "./components/ui";

// Log application startup
logInfo("SparrowAI Frontend Starting", {
  timestamp: new Date().toISOString(),
  userAgent: navigator.userAgent,
  environment: import.meta.env.MODE,
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <ErrorBoundary>
    <App />
  </ErrorBoundary>
);
