// React App Shell and Page Router
//
//   Top-level layout: site header plus Home (demo actions) or Dashboard (audit).
//   Minimal client-side routing via useState — no react-router.
//
// INPUT:
//   - onAudit callback from Home to switch pages
//
// OUTPUT:
//   - Rendered Home or Dashboard component inside .shell layout
//
// NOTES:
//   Clicking the site header returns to Home from Audit.
//
// Written by Composer for Ze Supply Chain. June 2025. All rights reserved.

import { useState } from "react";
import { Home } from "./Home";
import { Dashboard } from "./Dashboard";

export type Page = "home" | "audit";

export default function App() {
  const [page, setPage] = useState<Page>("home");

  return (
    <div className="shell">
      <header className="site" onClick={() => setPage("home")}>
        <h1>
          <span className="zec">ⓩ</span> ZE SUPPLY CHAIN
        </h1>
        <span className="sub">immutable audit log on Zcash mainnet</span>
      </header>
      {page === "home" ? (
        <Home onAudit={() => setPage("audit")} />
      ) : (
        <Dashboard />
      )}
    </div>
  );
}
