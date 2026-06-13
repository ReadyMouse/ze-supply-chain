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
