import { useState, useCallback, useEffect } from "react";
import { Input } from "@/components/ui/Input";
import { cn } from "@/utils/cn";
import { CID } from "multiformats/cid";

interface CidInputProps {
  value: string;
  onChange: (value: string, isValid: boolean, parsedCid?: CID) => void;
  placeholder?: string;
  className?: string;
  disabled?: boolean;
}

export function CidInput({
  value,
  onChange,
  placeholder = "Enter CID (bafk... or 0x...)",
  className,
  disabled,
}: CidInputProps) {
  const [error, setError] = useState<string | null>(null);

  const validateCid = useCallback((input: string): { isValid: boolean; cid?: CID; error?: string } => {
    const trimmed = input.trim();
    if (!trimmed) {
      return { isValid: false };
    }

    // Accept 0x-prefixed hex (raw CID bytes)
    if (trimmed.startsWith("0x") || trimmed.startsWith("0X")) {
      try {
        const hex = trimmed.slice(2);
        if (hex.length === 0 || hex.length % 2 !== 0) {
          return { isValid: false, error: "Invalid hex length" };
        }
        const bytes = new Uint8Array(hex.length / 2);
        for (let i = 0; i < hex.length; i += 2) {
          bytes[i / 2] = parseInt(hex.slice(i, i + 2), 16);
        }
        const cid = CID.decode(bytes);
        return { isValid: true, cid };
      } catch {
        return { isValid: false, error: "Invalid CID hex" };
      }
    }

    try {
      const cid = CID.parse(trimmed);
      return { isValid: true, cid };
    } catch {
      return { isValid: false, error: "Invalid CID format" };
    }
  }, []);

  // Validate on mount when value is pre-populated (e.g. from URL query param)
  useEffect(() => {
    if (value.trim()) {
      const result = validateCid(value);
      onChange(value, result.isValid, result.cid);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value;
    const result = validateCid(newValue);

    if (newValue && !result.isValid) {
      setError(result.error || "Invalid CID");
    } else {
      setError(null);
    }

    onChange(newValue, result.isValid, result.cid);
  };

  return (
    <div className={cn("space-y-1", className)}>
      <Input
        value={value}
        onChange={handleChange}
        placeholder={placeholder}
        disabled={disabled}
        className={cn(
          "font-mono text-sm",
          error && "border-destructive focus-visible:ring-destructive"
        )}
      />
      {error && (
        <p className="text-xs text-destructive">{error}</p>
      )}
    </div>
  );
}
