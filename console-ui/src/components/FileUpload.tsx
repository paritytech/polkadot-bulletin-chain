import { useCallback, useState } from "react";
import { Upload, File as FileIcon, FolderOpen, X, Plus } from "lucide-react";
import { Button } from "@/components/ui/Button";
import { cn } from "@/utils/cn";
import { formatBytes } from "@/utils/format";

export interface SelectedFile {
  file: File;
  data: Uint8Array;
  relativePath: string;
}

interface FileWithPath {
  file: File;
  relativePath: string;
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

// ── Directory traversal helpers ──────────────────────────────────────

async function readAllDirectoryEntries(
  reader: FileSystemDirectoryReader,
): Promise<FileSystemEntry[]> {
  const allEntries: FileSystemEntry[] = [];
  const readBatch = (): Promise<FileSystemEntry[]> =>
    new Promise((resolve, reject) => reader.readEntries(resolve, reject));

  // readEntries returns batches (Chrome limits to 100 per batch)
  let batch: FileSystemEntry[];
  do {
    batch = await readBatch();
    allEntries.push(...batch);
  } while (batch.length > 0);

  return allEntries;
}

async function collectFromEntry(
  entry: FileSystemEntry,
  parentPath: string,
  results: FileWithPath[],
): Promise<void> {
  const path = parentPath ? `${parentPath}/${entry.name}` : entry.name;

  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => {
      (entry as FileSystemFileEntry).file(resolve, reject);
    });
    results.push({ file, relativePath: path });
  } else if (entry.isDirectory) {
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    const children = await readAllDirectoryEntries(reader);
    for (const child of children) {
      await collectFromEntry(child, path, results);
    }
  }
}

async function collectDroppedFiles(
  dataTransfer: DataTransfer,
): Promise<FileWithPath[]> {
  const results: FileWithPath[] = [];

  // Try webkitGetAsEntry for directory support
  if (dataTransfer.items) {
    const entries: FileSystemEntry[] = [];
    for (let i = 0; i < dataTransfer.items.length; i++) {
      const entry = dataTransfer.items[i]?.webkitGetAsEntry?.();
      if (entry) entries.push(entry);
    }

    if (entries.length > 0) {
      for (const entry of entries) {
        await collectFromEntry(entry, "", results);
      }
      return results;
    }
  }

  // Fallback: plain files
  for (let i = 0; i < dataTransfer.files.length; i++) {
    const file = dataTransfer.files[i]!;
    results.push({ file, relativePath: file.name });
  }
  return results;
}

// ── Programmatic file/folder pickers ─────────────────────────────────

function openFilePicker(
  multiple: boolean,
  accept: string | undefined,
  onFiles: (files: FileWithPath[]) => void,
) {
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = multiple;
  if (accept) input.accept = accept;
  input.addEventListener("change", () => {
    const files = Array.from(input.files || []);
    if (files.length > 0) {
      onFiles(files.map(f => ({ file: f, relativePath: f.name })));
    }
  });
  input.click();
}

function openFolderPicker(onFiles: (files: FileWithPath[]) => void) {
  const input = document.createElement("input");
  input.type = "file";
  input.setAttribute("webkitdirectory", "");
  input.setAttribute("directory", "");
  input.addEventListener("change", () => {
    const files = Array.from(input.files || []);
    if (files.length === 0) return;
    // Strip the top-level folder prefix from webkitRelativePath
    const filesWithPaths = files.map(file => {
      const parts = file.webkitRelativePath.split("/");
      const relativePath = parts.slice(1).join("/") || file.name;
      return { file, relativePath };
    });
    onFiles(filesWithPaths);
  });
  input.click();
}

// ─────────────────────────────────────────────────────────────────────

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
  const processMultipleFiles = useCallback(async (newFiles: FileWithPath[]) => {
    setError(null);
    const validFiles: SelectedFile[] = [];

    for (const { file, relativePath } of newFiles) {
      if (maxSize && file.size > maxSize) {
        setError(`"${relativePath}" exceeds ${formatBytes(maxSize)} limit`);
        continue;
      }
      try {
        const arrayBuffer = await file.arrayBuffer();
        validFiles.push({ file, data: new Uint8Array(arrayBuffer), relativePath });
      } catch {
        setError(`Failed to read "${relativePath}"`);
      }
    }

    if (validFiles.length === 0) return;
    const updated = [...selectedFiles, ...validFiles];
    setSelectedFiles(updated);
    onFilesSelect?.(updated);
  }, [maxSize, selectedFiles, onFilesSelect]);

  const handleDrop = useCallback(async (e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
    if (disabled) return;

    if (multiple) {
      const filesWithPaths = await collectDroppedFiles(e.dataTransfer);
      if (filesWithPaths.length > 0) processMultipleFiles(filesWithPaths);
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

  const selectFiles = useCallback(() => {
    if (disabled) return;
    if (multiple) {
      openFilePicker(true, accept, processMultipleFiles);
    } else {
      openFilePicker(false, accept, (files) => {
        if (files[0]) processFile(files[0].file);
      });
    }
  }, [disabled, multiple, accept, processFile, processMultipleFiles]);

  const selectFolder = useCallback(() => {
    if (disabled) return;
    openFolderPicker(processMultipleFiles);
  }, [disabled, processMultipleFiles]);

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
        <div className="rounded-lg border bg-card divide-y max-h-60 overflow-y-auto">
          {selectedFiles.map((sf, index) => (
            <div
              key={`${sf.relativePath}-${index}`}
              className="flex items-center justify-between px-4 py-2"
            >
              <div className="flex items-center gap-3 min-w-0">
                <FileIcon className="h-5 w-5 text-muted-foreground shrink-0" />
                <div className="min-w-0">
                  <p className="font-medium text-sm truncate">{sf.relativePath}</p>
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

        {/* Add more files/folders drop zone */}
        <div
          onDrop={handleDrop}
          onDragOver={handleDragOver}
          onDragLeave={handleDragLeave}
          className={cn(
            "rounded-lg border-2 border-dashed p-3 text-center transition-colors",
            isDragging && "border-primary bg-primary/5",
            disabled && "opacity-50 cursor-not-allowed",
          )}
        >
          <div className="flex items-center justify-center gap-3">
            <button
              type="button"
              onClick={selectFiles}
              disabled={disabled}
              className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
            >
              <Plus className="h-4 w-4" />
              Add files
            </button>
            <span className="text-muted-foreground text-xs">or</span>
            <button
              type="button"
              onClick={selectFolder}
              disabled={disabled}
              className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
            >
              <FolderOpen className="h-4 w-4" />
              Add folder
            </button>
            <span className="text-muted-foreground text-xs">or drop here</span>
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
        onClick={selectFiles}
        onDrop={handleDrop}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        className={cn(
          "rounded-lg border-2 border-dashed p-8 text-center transition-colors",
          isDragging && "border-primary bg-primary/5",
          disabled && "opacity-50 cursor-not-allowed",
          !disabled && "cursor-pointer hover:border-primary/50"
        )}
      >
        <Upload className="h-10 w-10 mx-auto text-muted-foreground mb-4" />
        <p className="text-sm text-muted-foreground">
          {multiple
            ? "Drag and drop files or folders here, or click to select files"
            : "Drag and drop a file here, or click to select"}
        </p>
        <p className="text-xs text-muted-foreground mt-1">
          Max size per file: {formatBytes(maxSize)}
        </p>
      </div>
      {multiple && (
        <button
          type="button"
          onClick={selectFolder}
          disabled={disabled}
          className={cn(
            "w-full rounded-lg border-2 border-dashed p-3 text-center transition-colors",
            disabled && "opacity-50 cursor-not-allowed",
            !disabled && "cursor-pointer hover:border-primary/50",
          )}
        >
          <span className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
            <FolderOpen className="h-4 w-4" />
            Or select a folder
          </span>
        </button>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
    </div>
  );
}
