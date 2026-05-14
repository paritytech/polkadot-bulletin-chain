import { AlertCircle } from "lucide-react";

export function PalletUnavailableNotice({
  pallet,
  details,
}: {
  pallet: string;
  details?: string;
}) {
  return (
    <div className="flex items-start gap-2 text-xs text-amber-600 bg-amber-500/10 p-3 rounded-md">
      <AlertCircle className="h-4 w-4 mt-0.5 shrink-0" />
      <div>
        <p className="font-medium">{pallet} pallet unavailable</p>
        {details && <p className="text-muted-foreground break-all">{details}</p>}
      </div>
    </div>
  );
}
