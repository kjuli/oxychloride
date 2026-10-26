// The metamodel diagram's page (see diagram/App.tsx; its host is src/diagram.ts).

import "@xyflow/react/dist/style.css";
import { createRoot } from "react-dom/client";
import { App } from "./diagram/App";

createRoot(document.getElementById("app")!).render(<App />);
