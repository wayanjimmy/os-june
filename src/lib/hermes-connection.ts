import type { HermesBridgeConnection, HermesBridgeStatus } from "./tauri";

const gatewayModes = new WeakMap<object, boolean>();

/** Records and reads the native connection mode that actually serves a gateway. */
export function rememberHermesGatewayMode(gateway: object, fullMode: boolean) {
  gatewayModes.set(gateway, fullMode);
}

export function hermesGatewayFullMode(gateway: object): boolean | undefined {
  return gatewayModes.get(gateway);
}

/** The live connection serving the requested effective write-access mode, if
 * any. Supported platforms can run one process per mode; Windows maps both
 * compatibility aliases to its sole Full-mode process. */
export function hermesConnectionForMode(
  status: HermesBridgeStatus | undefined,
  fullMode: boolean,
): HermesBridgeConnection | undefined {
  if (!status?.running) return undefined;
  // Windows exposes one canonical Full-mode process. Both legacy mode aliases
  // resolve to it, while supported platforms remain strict and never fall back
  // to the wrong process.
  const effectiveFullMode = status.sandboxModeSupported === false ? true : fullMode;
  const fromList = status.connections?.find(
    (connection) => Boolean(connection.fullMode) === effectiveFullMode,
  );
  if (fromList) return fromList;
  // Older payload shape without `connections`.
  return status.connection && Boolean(status.connection.fullMode) === effectiveFullMode
    ? status.connection
    : undefined;
}
