import { useCallback, useState } from "react";
import { Upload, File as FileIcon, X, Plus } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { cn } from "@/utils/cn";
import { formatBytes } from "@/utils/format";

export interface SelectedFile {
  file: File;
  data: Uint8Array;
}

interface FileUploadProps {
  onFileSelect?: (file: File | null, data: Uint8Array | null) => void;
  onFilesSelect?: (files: SelectedFile[]) => void;
  multiple?: boolean;
  maxSize?: number; // per file, in bytes
  accept?: string;
  className?: string;
  disabled?: boolean;
}

export function FileUpload({
  onFileSelect,
  onFilesSelect,
  multiple = false,
  maxSize = 1024 * 1024, // 1MB default
  accept,
  className,
  disabled,
}: FileUploadProps) {
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [selectedFiles, setSelectedFiles] = useState<SelectedFile[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  // Single-file processing (existing behavior)
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
      onFileSelect?.(file, data);
    } catch {
      setError("Failed to read file");
    }
  }, [maxSize, onFileSelect]);

  // Multi-file processing: validates each file and appends to list
  const processMultipleFiles = useCallback(async (newFiles: File[]) => {
    setError(null);
    const validFiles: SelectedFile[] = [];

    for (const file of newFiles) {
      if (maxSize && file.size > maxSize) {
        setError(`"${file.name}" exceeds ${formatBytes(maxSize)} limit`);
        continue;
      }
      try {
        const arrayBuffer = await file.arrayBuffer();
        validFiles.push({ file, data: new Uint8Array(arrayBuffer) });
      } catch {
        setError(`Failed to read "${file.name}"`);
      }
    }

    if (validFiles.length === 0) return;
    const updated = [...selectedFiles, ...validFiles];
    setSelectedFiles(updated);
    onFilesSelect?.(updated);
  }, [maxSize, selectedFiles, onFilesSelect]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    if (disabled) return;

    if (multiple) {
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) processMultipleFiles(files);
    } else {
      const file = e.dataTransfer.files[0];
      if (file) processFile(file);
    }
  }, [disabled, multiple, processFile, processMultipleFiles]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled) setIsDragging(true);
  }, [disabled]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  }, []);

  const handleFileInput = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    if (multiple) {
      const files = Array.from(e.target.files || []);
      if (files.length > 0) processMultipleFiles(files);
    } else {
      const file = e.target.files?.[0];
      if (file) processFile(file);
    }
    e.target.value = "";
  }, [multiple, processFile, processMultipleFiles]);

  const clearFile = useCallback(() => {
    setSelectedFile(null);
    setError(null);
    onFileSelect?.(null, null);
  }, [onFileSelect]);

  const removeFile = useCallback((index: number) => {
    const updated = selectedFiles.filter((_, i) => i !== index);
    setSelectedFiles(updated);
    onFilesSelect?.(updated);
  }, [selectedFiles, onFilesSelect]);

  const clearAllFiles = useCallback(() => {
    setSelectedFiles([]);
    setError(null);
    onFilesSelect?.([]);
  }, [onFilesSelect]);

  // ─── Single file selected (non-multiple mode) ─────────────────────
  if (!multiple && selectedFile) {
    return (
      <div className={cn("rounded-lg border bg-card p-4", className)}>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <FileIcon className="h-8 w-8 text-muted-foreground" />
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

  // ─── Multiple files selected ───────────────────────────────────────
  if (multiple && selectedFiles.length > 0) {
    const totalSize = selectedFiles.reduce((sum, f) => sum + f.data.length, 0);

    return (
      <div className={cn("space-y-3", className)}>
        {/* File list */}
        <div className="rounded-lg border bg-card divide-y">
          {selectedFiles.map((sf, index) => (
            <div
              key={`${sf.file.name}-${index}`}
              className="flex items-center justify-between px-4 py-2"
            >
              <div className="flex items-center gap-3 min-w-0">
                <FileIcon className="h-5 w-5 text-muted-foreground shrink-0" />
                <div className="min-w-0">
                  <p className="font-medium text-sm truncate">{sf.file.name}</p>
                  <p className="text-xs text-muted-foreground">
                    {formatBytes(sf.file.size)}
                  </p>
                </div>
              </div>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => removeFile(index)}
                disabled={disabled}
                className="shrink-0 h-8 w-8"
              >
                <X className="h-3 w-3" />
              </Button>
            </div>
          ))}
        </div>

        {/* Summary */}
        <div className="flex items-center justify-between text-sm">
          <span className="text-muted-foreground">
            {selectedFiles.length} file{selectedFiles.length !== 1 ? "s" : ""} —{" "}
            {formatBytes(totalSize)} total
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={clearAllFiles}
            disabled={disabled}
          >
            Clear all
          </Button>
        </div>

        {/* Add more files drop zone */}
        <div
          onDrop={handleDrop}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          className={cn(
            "relative rounded-lg border-2 border-dashed p-4 text-center transition-colors",
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
            multiple
            className="absolute inset-0 w-full h-full opacity-0 cursor-pointer disabled:cursor-not-allowed"
          />
          <div className="flex items-center justify-center gap-2">
            <Plus className="h-4 w-4 text-muted-foreground" />
            <span className="text-sm text-muted-foreground">
              Add more files
            </span>
          </div>
        </div>

        {error && <p className="text-sm text-destructive">{error}</p>}
      </div>
    );
  }

  // ─── Empty state: drop zone ────────────────────────────────────────
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
          multiple={multiple}
          className="absolute inset-0 w-full h-full opacity-0 cursor-pointer disabled:cursor-not-allowed"
        />
        <Upload className="h-10 w-10 mx-auto text-muted-foreground mb-4" />
        <p className="text-sm text-muted-foreground">
          {multiple
            ? "Drag and drop files here, or click to select"
            : "Drag and drop a file here, or click to select"}
        </p>
        <p className="text-xs text-muted-foreground mt-1">
          Max size per file: {formatBytes(maxSize)}
        </p>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}
