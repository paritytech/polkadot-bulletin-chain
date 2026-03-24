import { useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { Subscribe } from "@react-rxjs/core";
import { Header } from "@/components/Header";
import { Dashboard } from "@/pages/Dashboard/Dashboard";
import { Upload } from "@/pages/Upload/Upload";
import { Download } from "@/pages/Download/Download";
import { Renew } from "@/pages/Renew/Renew";
import { Explorer } from "@/pages/Explorer/Explorer";
import { Authorizations } from "@/pages/Authorizations/Authorizations";
import { Accounts } from "@/pages/Accounts/Accounts";
import { restoreWalletConnection } from "@/state/wallet.state";

export default function App() {
  useEffect(() => {
    restoreWalletConnection();
  }, []);
  return (
    <Subscribe>
      <div className="min-h-screen flex flex-col">
        <Header />
        <main className="flex-1 container mx-auto px-4 py-6 max-w-7xl">
          <Routes>
            <Route path="/" element={<Dashboard />} />
            <Route path="/upload" element={<Upload />} />
            <Route path="/download" element={<Download />} />
            <Route path="/renew" element={<Renew />} />
            <Route path="/explorer" element={<Explorer />} />
            <Route path="/authorizations" element={<Authorizations />} />
            <Route path="/accounts" element={<Accounts />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </main>
      </div>
    </Subscribe>
  );
}
