import { useEffect, useRef, useState } from "react";
import * as Tooltip from "@radix-ui/react-tooltip";
import { Check, Copy } from "lucide-react";

export function IconButton({ label, onClick, children, ...props }) {
  return (
    <Tooltip.Root>
      <Tooltip.Trigger asChild>
        <button type="button" className="gb-icon-button" aria-label={label} onClick={onClick} {...props}>{children}</button>
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content className="gb-tooltip" sideOffset={6}>{label}</Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}

export function useCopied() {
  const [copied, setCopied] = useState(false);
  const timer = useRef();
  useEffect(() => () => clearTimeout(timer.current), []);
  return [copied, () => {
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1800);
  }];
}

export function CopyIconButton({ text, label, onCopy }) {
  const [copied, markCopied] = useCopied();
  return (
    <IconButton label={copied ? "Copié" : label} onClick={async () => { if (await onCopy(text)) markCopied(); }}>
      {copied ? <Check /> : <Copy />}
    </IconButton>
  );
}
