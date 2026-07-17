import { IconGoogle } from "central-icons/IconGoogle";
import { IconLinear } from "central-icons/IconLinear";
import { IconVault } from "central-icons/IconVault";

/** Brand mark for a connector provider. Shared by connector settings and approvals. */
export function ConnectorProviderIcon({
  provider,
  size = 18,
}: {
  provider: "google" | "linear" | "obsidian";
  size?: number;
}) {
  if (provider === "obsidian") {
    return <IconVault size={size} aria-hidden />;
  }
  if (provider === "linear") {
    return <IconLinear size={size} aria-hidden />;
  }
  return <IconGoogle size={size} aria-hidden />;
}
