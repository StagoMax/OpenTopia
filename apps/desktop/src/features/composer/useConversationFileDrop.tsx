import {
  useCallback,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";
import { Paperclip } from "lucide-react";
import { hasFileDragPayload } from "../../fileDrop";

export type ComposerFileDropHandle = {
  addFiles(files: File[]): void;
};

export function useConversationFileDrop(receiverRef: {
  readonly current: ComposerFileDropHandle | null;
}) {
  const dragDepthRef = useRef(0);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);

  const resetDragState = useCallback(() => {
    dragDepthRef.current = 0;
    setIsDraggingFiles(false);
  }, []);

  const onDragEnter = useCallback(
    (event: ReactDragEvent<HTMLElement>) => {
      if (
        !receiverRef.current ||
        !hasFileDragPayload(event.dataTransfer.types)
      ) {
        return;
      }
      event.preventDefault();
      dragDepthRef.current += 1;
      setIsDraggingFiles(true);
    },
    [receiverRef],
  );

  const onDragOver = useCallback(
    (event: ReactDragEvent<HTMLElement>) => {
      if (
        !receiverRef.current ||
        !hasFileDragPayload(event.dataTransfer.types)
      ) {
        return;
      }
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    },
    [receiverRef],
  );

  const onDragLeave = useCallback(() => {
    dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
    if (dragDepthRef.current === 0) setIsDraggingFiles(false);
  }, []);

  const onDrop = useCallback(
    (event: ReactDragEvent<HTMLElement>) => {
      if (!hasFileDragPayload(event.dataTransfer.types)) return;
      event.preventDefault();
      const receiver = receiverRef.current;
      resetDragState();
      receiver?.addFiles(Array.from(event.dataTransfer.files));
    },
    [receiverRef, resetDragState],
  );

  return { isDraggingFiles, onDragEnter, onDragOver, onDragLeave, onDrop };
}

export function ConversationFileDropTarget() {
  return (
    <div className="conversation-drop-target" aria-hidden="true">
      <Paperclip size={20} />
      <span>释放以添加文件</span>
    </div>
  );
}
