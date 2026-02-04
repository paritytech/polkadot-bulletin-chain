import { useState, useCallback } from "react";
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
  placeholder = "Enter CID (e.g., bafk...)",
  className,
  disabled,
}: CidInputProps) {
  const [error, setError] = useState<string | null>(null);

  const validateCid = useCallback((input: string): { isValid: boolean; cid?: CID; error?: string } => {
    if (!input.trim()) {
      return { isValid: false };
    }

    try {
      const cid = CID.parse(input.trim());
      return { isValid: true, cid };
    } catch {
      return { isValid: false, error: "Invalid CID format" };
    }
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
