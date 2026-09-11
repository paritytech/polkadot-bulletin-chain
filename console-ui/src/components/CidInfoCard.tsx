// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-only

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { CID } from "@parity/bulletin-sdk";
import { codecLabel, hashLabel } from "@/lib/cid-labels";
import { contentHashHex } from "@/lib/cid-lookup";

function CopyButton({ value, label }: { value: string; label: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard API blocked — silently ignore.
    }
  };

  return (
    <button
      type="button"
      onClick={handleCopy}
      className="text-muted-foreground hover:text-foreground transition-colors"
      title={`Copy ${label}`}
      aria-label={`Copy ${label}`}
    >
      {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
    </button>
  );
}

function HexRow({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="flex items-center justify-between">
        <span className="text-muted-foreground">{label}</span>
        <CopyButton value={value} label={label} />
      </div>
      <p className="font-mono text-xs mt-1 break-all">{value}</p>
    </div>
  );
}

export function CidInfoCard({ cid }: { cid: CID | undefined }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>CID Info</CardTitle>
        <CardDescription>Parsed CID details</CardDescription>
      </CardHeader>
      <CardContent>
        {cid ? (
          <div className="space-y-3 text-sm">
            <HexRow label="CID" value={cid.toString()} />
            <div className="flex justify-between">
              <span className="text-muted-foreground">Version</span>
              <Badge variant="secondary">CIDv{cid.version}</Badge>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Codec</span>
              <span className="font-mono">{codecLabel(cid.code)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Hash</span>
              <span className="font-mono">{hashLabel(cid.multihash.code)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Hash Size</span>
              <span>{cid.multihash.size} bytes</span>
            </div>
            <HexRow label="Content Hash" value={contentHashHex(cid)} />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">Enter a valid CID to see details</p>
        )}
      </CardContent>
    </Card>
  );
}
