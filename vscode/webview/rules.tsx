// The rule diagram's page (see rules/App.tsx; its host is src/ruleDiagram.ts).

import "@xyflow/react/dist/style.css";
import { createRoot } from "react-dom/client";
import { App } from "./rules/App";

createRoot(document.getElementById("app")!).render(<App />);
