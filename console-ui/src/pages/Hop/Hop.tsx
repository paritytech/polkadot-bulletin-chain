// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { useCallback, useEffect, useRef, useState } from "react";
import { Boxes, RefreshCw, Plus, X, AlertTriangle, Settings2 } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Badge } from "@/components/ui/Badge";
import { Spinner } from "@/components/ui/Spinner";
import { useChainState } from "@/state/chain.state";
import {
  fetchHopPoolStatus,
  getHopRefreshSecs,
  setHopRefreshSecs,
  type HopPoolStatus,
} from "@/state/hop.state";
import { formatBytes, formatNumber, formatTimestamp } from "@/utils/format";

interface NodeResult {
  status: "idle" | "loading" | "ok" | "error";
  data?: HopPoolStatus;
  error?: string;
  fetchedAt?: number;
}

function usagePercent(s: HopPoolStatus): number {
  if (s.maxBytes <= 0) return 0;
  return (s.totalBytes / s.maxBytes) * 100;
}

function StatusBadge({ result }: { result: NodeResult | undefined }) {
  if (!result || result.status === "idle") return <Badge variant="secondary">—</Badge>;
  if (result.status === "loading") return <Badge variant="warning">Loading…</Badge>;
  if (result.status === "error") return <Badge variant="destructive">Error</Badge>;
  return <Badge variant="success">OK</Badge>;
}

function NodeRow({ url, result }: { url: string; result: NodeResult | undefined }) {
  const data = result?.data;
  return (
    <tr className="border-b last:border-0 align-top">
      <td className="py-2 pr-4 font-mono text-xs break-all">{url}</td>
      <td className="py-2 pr-4">
        <StatusBadge result={result} />
      </td>
      <td className="py-2 pr-4 text-right font-mono">
        {data ? formatNumber(data.entryCount) : "—"}
      </td>
      <td className="py-2 pr-4 text-right font-mono">
        {data ? formatBytes(data.totalBytes) : "—"}
      </td>
      <td className="py-2 pr-4 text-right font-mono">
        {data ? formatBytes(data.maxBytes) : "—"}
      </td>
      <td className="py-2 pr-4 text-right font-mono">
        {data ? `${usagePercent(data).toFixed(2)}%` : "—"}
      </td>
      <td className="py-2 text-xs text-muted-foreground">
        {result?.status === "error" ? (
          <span className="text-destructive break-all">{result.error}</span>
        ) : result?.fetchedAt ? (
          formatTimestamp(result.fetchedAt)
        ) : (
          "—"
        )}
      </td>
    </tr>
  );
}

function ConfigCard({
  nodes,
  setNodes,
  intervalSecs,
  setIntervalSecs,
  autoRefresh,
  setAutoRefresh,
}: {
  nodes: string[];
  setNodes: (n: string[]) => void;
  intervalSecs: number;
  setIntervalSecs: (n: number) => void;
  autoRefresh: boolean;
  setAutoRefresh: (b: boolean) => void;
}) {
  const updateNode = (i: number, value: string) => {
    const next = [...nodes];
    next[i] = value;
    setNodes(next);
  };
  const removeNode = (i: number) => setNodes(nodes.filter((_, idx) => idx !== i));
  const addNode = () => setNodes([...nodes, ""]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Settings2 className="h-5 w-5" />
          HOP Nodes
        </CardTitle>
        <CardDescription>
          Seeded from the selected network. Add ad-hoc endpoints below for this session.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-2">
          {nodes.map((url, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                value={url}
                onChange={(e) => updateNode(i, e.target.value)}
                placeholder="https://… or wss://…"
                className="font-mono text-xs"
                spellCheck={false}
              />
              <Button
                variant="ghost"
                size="icon"
                onClick={() => removeNode(i)}
                title="Remove node"
                className="shrink-0"
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
          ))}
          <Button variant="outline" size="sm" onClick={addNode}>
            <Plus className="h-4 w-4 mr-1.5" />
            Add node
          </Button>
        </div>

        <div className="mt-6 flex flex-wrap items-end gap-6">
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
              className="h-4 w-4"
            />
            Auto-refresh
          </label>
          <div className="space-y-1">
            <p className="text-xs text-muted-foreground">Interval (seconds)</p>
            <Input
              type="number"
              min={1}
              value={intervalSecs}
              onChange={(e) => setIntervalSecs(Math.max(1, Number(e.target.value) || 1))}
              className="w-28"
            />
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

export function Hop() {
  const { network } = useChainState();

  const [nodes, setNodes] = useState<string[]>(() => network.hopNodes ?? []);
  const [intervalSecs, setIntervalSecsState] = useState<number>(() => getHopRefreshSecs());
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [results, setResults] = useState<Record<string, NodeResult>>({});

  const nodesRef = useRef(nodes);
  nodesRef.current = nodes;

  const setIntervalSecs = useCallback((secs: number) => {
    setIntervalSecsState(secs);
    setHopRefreshSecs(secs);
  }, []);

  const refreshAll = useCallback(async (urlsArg?: string[]) => {
    const urls = (urlsArg ?? nodesRef.current).map((u) => u.trim()).filter(Boolean);
    setResults((prev) => {
      const next: Record<string, NodeResult> = {};
      for (const url of urls) next[url] = { ...prev[url], status: "loading" };
      return next;
    });
    await Promise.all(
      urls.map(async (url) => {
        try {
          const data = await fetchHopPoolStatus(url);
          setResults((prev) => ({ ...prev, [url]: { status: "ok", data, fetchedAt: Date.now() } }));
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          setResults((prev) => ({
            ...prev,
            [url]: { status: "error", error: message, fetchedAt: Date.now() },
          }));
        }
      }),
    );
  }, []);

  // Re-seed the node list from the selected network (and refresh) on switch.
  useEffect(() => {
    const next = network.hopNodes ?? [];
    setNodes(next);
    setResults({});
    refreshAll(next);
  }, [network.id, refreshAll]);

  // Auto-refresh on the configured interval.
  useEffect(() => {
    if (!autoRefresh || intervalSecs <= 0) return;
    const id = setInterval(() => refreshAll(), intervalSecs * 1000);
    return () => clearInterval(id);
  }, [autoRefresh, intervalSecs, refreshAll]);

  const anyLoading = Object.values(results).some((r) => r.status === "loading");
  const activeUrls = nodes.map((u) => u.trim()).filter(Boolean);

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">HOP Pool Status</h1>
          <p className="text-muted-foreground">
            Live <code>hop_poolStatus</code> from <strong>{network.name}</strong> HOP relay nodes
          </p>
        </div>
        <Button onClick={() => refreshAll()} disabled={anyLoading}>
          <RefreshCw className={anyLoading ? "h-4 w-4 mr-2 animate-spin" : "h-4 w-4 mr-2"} />
          Refresh
        </Button>
      </div>

      <ConfigCard
        nodes={nodes}
        setNodes={setNodes}
        intervalSecs={intervalSecs}
        setIntervalSecs={setIntervalSecs}
        autoRefresh={autoRefresh}
        setAutoRefresh={setAutoRefresh}
      />

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Boxes className="h-5 w-5" />
            Pool Status
            {anyLoading && <Spinner size="sm" className="text-muted-foreground" />}
          </CardTitle>
          <CardDescription>
            <code>totalBytes</code> is accounted bytes (blob + 40 B/recipient), not raw disk usage.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {activeUrls.length === 0 ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground py-4">
              <AlertTriangle className="h-4 w-4" />
              {network.name} has no HOP nodes configured. Add one above, or switch network.
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-left text-xs uppercase tracking-wide text-muted-foreground">
                    <th className="py-2 pr-4 font-medium">Node</th>
                    <th className="py-2 pr-4 font-medium">Status</th>
                    <th className="py-2 pr-4 font-medium text-right">Entries</th>
                    <th className="py-2 pr-4 font-medium text-right">Accounted</th>
                    <th className="py-2 pr-4 font-medium text-right">Capacity</th>
                    <th className="py-2 pr-4 font-medium text-right">Usage</th>
                    <th className="py-2 font-medium">Updated</th>
                  </tr>
                </thead>
                <tbody>
                  {activeUrls.map((url) => (
                    <NodeRow key={url} url={url} result={results[url]} />
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
