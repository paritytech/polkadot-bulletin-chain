import { Activity, BarChart3, BookOpen, ExternalLink, Globe, LineChart, ScrollText } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { useChainState } from "@/state/chain.state";
import type { MonitoringLinks } from "@/config/networks";

type LinkSpec = {
  label: string;
  href: string;
  icon: typeof Activity;
  description: string;
};

type Subgroup = { title: string; items: LinkSpec[] };
type Group    = { title: string; items?: LinkSpec[]; subgroups?: Subgroup[] };

function group(monitoring: MonitoringLinks | undefined): Group[] {
  if (!monitoring) return [];

  const chainHealth: LinkSpec[] = [];
  if (monitoring.grafana) {
    chainHealth.push({
      label: "Grafana — Operation Health",
      href: monitoring.grafana,
      icon: Activity,
      description: "Block production, finality, peer count.",
    });
  }
  if (monitoring.collatorLogs) {
    chainHealth.push({
      label: "Collator Logs",
      href: monitoring.collatorLogs,
      icon: ScrollText,
      description: "Live log stream from collator nodes (Grafana Loki).",
    });
  }
  if (monitoring.telemetry) {
    chainHealth.push({
      label: "Collators/Nodes",
      href: monitoring.telemetry,
      icon: Globe,
      description: "Live list of every node running this chain (version, block height, peers).",
    });
  }
  if (monitoring.polkadotJs) {
    chainHealth.push({
      label: "PolkadotJS Apps",
      href: monitoring.polkadotJs,
      icon: ExternalLink,
      description: "Inspect chain state and events.",
    });
  }
  if (monitoring.explorer) {
    chainHealth.push({
      label: "Block Explorer",
      href: monitoring.explorer,
      icon: BarChart3,
      description: "Browse blocks and extrinsics.",
    });
  }

  // Writes, ordered by relevance: per-chunk SLI source first, then per-deploy
  // parent, then the full dashboard for context.
  const writes: LinkSpec[] = [];
  if (monitoring.sentryChunkUploadSpan) {
    writes.push({
      label: "deploy.chunk-upload span",
      href: monitoring.sentryChunkUploadSpan,
      icon: LineChart,
      description: "Per-chunk submit-to-finalized latency. Primary write SLI source.",
    });
  }
  if (monitoring.sentryStorageSpan) {
    writes.push({
      label: "deploy.storage span",
      href: monitoring.sentryStorageSpan,
      icon: LineChart,
      description: "Per-deploy Bulletin storage phase (wraps all chunks).",
    });
  }
  if (monitoring.sentry) {
    writes.push({
      label: "Bulletin Deploy Health",
      href: monitoring.sentry,
      icon: LineChart,
      description: "Full deploy dashboard.",
    });
  }

  const reads: LinkSpec[] = [];
  if (monitoring.sentryChainProbeSpan) {
    reads.push({
      label: "deploy.chain-probe span",
      href: monitoring.sentryChainProbeSpan,
      icon: LineChart,
      description: "Cache-check RPC reads against the chain.",
    });
  }

  const sentrySubgroups: Subgroup[] = [];
  if (writes.length) sentrySubgroups.push({ title: "Write", items: writes });
  if (reads.length)  sentrySubgroups.push({ title: "Read",  items: reads });

  const docs: LinkSpec[] = [];
  if (monitoring.runbook) {
    docs.push({
      label: "Runbook",
      href: monitoring.runbook,
      icon: BookOpen,
      description: "Operational playbook.",
    });
  }

  const groups: Group[] = [
    { title: "Chain Health", items: chainHealth },
    { title: "Sentry",       subgroups: sentrySubgroups },
    { title: "Docs",         items: docs },
  ];
  return groups.filter((g) =>
    (g.items?.length ?? 0) > 0 || (g.subgroups?.length ?? 0) > 0,
  );
}

function LinkCard({ link }: { link: LinkSpec }) {
  return (
    <a
      href={link.href}
      target="_blank"
      rel="noopener noreferrer"
      className="block hover:shadow-md transition-shadow"
    >
      <Card className="h-full">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <link.icon className="h-4 w-4" />
            {link.label}
            <ExternalLink className="h-3 w-3 text-muted-foreground" />
          </CardTitle>
          <CardDescription>{link.description}</CardDescription>
        </CardHeader>
      </Card>
    </a>
  );
}

function LinkGrid({ items }: { items: LinkSpec[] }) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3">
      {items.map((link) => (
        <LinkCard key={link.label} link={link} />
      ))}
    </div>
  );
}

export function Ops() {
  const { network } = useChainState();
  const groups = group(network?.monitoring);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <Activity className="h-6 w-6" />
            Operations &amp; Diagnostics
          </h1>
          <p className="text-sm text-muted-foreground">
            External dashboards and telemetry for the selected network.
          </p>
        </div>
        {network && <Badge variant="secondary">{network.name}</Badge>}
      </div>

      {groups.length === 0 ? (
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            No monitoring links configured for this network.
          </CardContent>
        </Card>
      ) : (
        groups.map((g) => (
          <section key={g.title} className="space-y-3">
            <h2 className="text-sm font-semibold text-muted-foreground uppercase tracking-wide">
              {g.title}
            </h2>
            {g.items && <LinkGrid items={g.items} />}
            {g.subgroups?.map((sg) => (
              <div key={sg.title} className="space-y-2">
                <h3 className="text-xs font-medium text-muted-foreground/80 ml-1">
                  {sg.title}
                </h3>
                <LinkGrid items={sg.items} />
              </div>
            ))}
          </section>
        ))
      )}
    </div>
  );
}
