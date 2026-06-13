// React Application Bootstrap
//
//   Mounts the root App component into #root with StrictMode and global styles.
//
// INPUT:
//   - index.html #root element
//
// OUTPUT:
//   - Rendered React application in the browser
//
// NOTES:
//   Entry point for Vite; imports styles.css globally.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
