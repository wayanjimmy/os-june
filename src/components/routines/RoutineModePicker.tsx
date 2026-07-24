import { IconShieldCheck } from "central-icons/IconShieldCheck";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { useId } from "react";
import { SegmentedControl } from "../ui/SegmentedControl";

const MODE_OPTIONS = [
  {
    value: "sandboxed",
    label: (
      <>
        <IconShieldCheck size={14} aria-hidden />
        Sandboxed
      </>
    ),
    ariaLabel: "Sandboxed",
  },
  {
    value: "unrestricted",
    label: (
      <>
        <IconShieldCrossed size={14} aria-hidden />
        Unrestricted
      </>
    ),
    ariaLabel: "Unrestricted",
  },
] as const;

type RoutineModePickerProps =
  | {
      availability: "supported";
      unrestricted: boolean;
      onChange: (unrestricted: boolean) => void;
    }
  | {
      availability: "unsupported" | "checking";
    };

/** The per-routine sandbox choice. Like the chat picker, Unrestricted is a
 * deliberate opt-in per routine, never a sticky preference. */
export function RoutineModePicker(props: RoutineModePickerProps) {
  const statusId = useId();

  if (props.availability === "checking") {
    return (
      <p className="routines-mode-hint" role="status">
        Checking access options...
      </p>
    );
  }

  const supported = props.availability === "supported";
  const unrestricted = supported ? props.unrestricted : true;

  return (
    <>
      <SegmentedControl
        value={unrestricted ? "unrestricted" : "sandboxed"}
        onValueChange={(value) => {
          if (supported) props.onChange(value === "unrestricted");
        }}
        options={MODE_OPTIONS}
        disabled={!supported}
        aria-describedby={supported ? undefined : statusId}
        // The indicator goes terracotta while Unrestricted is armed, same
        // warm accent as the composer's sandbox trigger.
        className={unrestricted ? "segmented-warm" : undefined}
        aria-label="What can this routine change?"
      />
      <p
        id={supported ? undefined : statusId}
        className="routines-mode-hint"
        role={supported ? undefined : "status"}
      >
        {!supported
          ? "Sandboxed mode is not supported on Windows. This routine will run with full access to files available to your Windows account."
          : unrestricted
            ? "When it fires, June can run commands and change any file your account can."
            : "The routine can read the web, use memory, and message you. It cannot run commands or change your files."}
      </p>
    </>
  );
}
