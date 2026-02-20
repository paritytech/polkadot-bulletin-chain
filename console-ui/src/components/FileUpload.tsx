import { useCallback, useState } from "react";
import { Upload, File, X } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { cn } from "@/utils/cn";
import { formatBytes } from "@/utils/format";

interface FileUploadProps {
  onFileSelect: (file: File | null, data: Uint8Array | null) => void;
  maxSize?: number; // in bytes
  accept?: string;
  className?: string;
  disabled?: boolean;
}

export function FileUpload({
  onFileSelect,
  maxSize = 1024 * 1024, // 1MB default
  accept,
  className,
  disabled,
}: FileUploadProps) {
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  const processFile = useCallback(async (file: File) => {
    setError(null);

    if (maxSize && file.size > maxSize) {
      setError(`File too large. Maximum size: ${formatBytes(maxSize)}`);
      return;
    }

    try {
      const arrayBuffer = await file.arrayBuffer();
      const data = new Uint8Array(arrayBuffer);
      setSelectedFile(file);
      onFileSelect(file, data);
    } catch {
      setError("Failed to read file");
    }
  }, [maxSize, onFileSelect]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);

    if (disabled) return;

    const file = e.dataTransfer.files[0];
    if (file) {
      processFile(file);
    }
  }, [disabled, processFile]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled) {
      setIsDragging(true);
    }
  }, [disabled]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  }, []);

  const handleFileInput = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      processFile(file);
    }
  }, [processFile]);

  const clearFile = useCallback(() => {
    setSelectedFile(null);
    setError(null);
    onFileSelect(null, null);
  }, [onFileSelect]);

  if (selectedFile) {
    return (
      <div className={cn("rounded-lg border bg-card p-4", className)}>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <File className="h-8 w-8 text-muted-foreground" />
            <div>
              <p className="font-medium text-sm">{selectedFile.name}</p>
              <p className="text-xs text-muted-foreground">
                {formatBytes(selectedFile.size)}
              </p>
            </div>
          </div>
          <Button
            variant="ghost"
            size="icon"
            onClick={clearFile}
            disabled={disabled}
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className={cn("space-y-2", className)}>
      <div
        onDrop={handleDrop}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        className={cn(
          "relative rounded-lg border-2 border-dashed p-8 text-center transition-colors",
          isDragging && "border-primary bg-primary/5",
          disabled && "opacity-50 cursor-not-allowed",
          !disabled && "cursor-pointer hover:border-primary/50"
        )}
      >
        <input
          type="file"
          onChange={handleFileInput}
          accept={accept}
          disabled={disabled}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer disabled:cursor-not-allowed"
        />
        <Upload className="h-10 w-10 mx-auto text-muted-foreground mb-4" />
        <p className="text-sm text-muted-foreground">
          Drag and drop a file here, or click to select
        </p>
        <p className="text-xs text-muted-foreground mt-1">
          Max size: {formatBytes(maxSize)}
        </p>
      </div>
      {error && (
        <p className="text-sm text-destructive">{error}</p>
      )}
    </div>
  );
}
