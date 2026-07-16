import { IconGoogle } from "central-icons/IconGoogle";
import obsidianLogo from "../../assets/obsidian-logo.svg";

/** Brand mark for a connector provider. Central-icons supplies available
 * marks; Obsidian's official favicon is vendored because central-icons does not
 * include that third-party brand. Shared by connector settings and approvals. */
export function ConnectorProviderIcon({
  provider,
  size = 18,
}: {
  provider: "google" | "obsidian";
  size?: number;
}) {
  if (provider === "obsidian") {
    return (
      <img
        className="connector-provider-icon-obsidian"
        src={obsidianLogo}
        width={size}
        height={size}
        alt=""
        aria-hidden
      />
    );
  }
  return <IconGoogle size={size} aria-hidden />;
}
