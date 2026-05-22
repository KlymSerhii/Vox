import { type Period } from "@/lib/ipc";

export const PERIOD_OPTIONS: { value: Period; label: string }[] = [
  { value: "last_session", label: "Last session" },
  { value: "day", label: "Today" },
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
  { value: "year", label: "Year" },
];

/// Donation page — a subpage of the rizenhq.com landing site. Opened via
/// `openExternalUrl` from the tray popover's permanent "Support Vox" row
/// and the milestone nudge's "Support Vox" button.
export const DONATE_URL = "https://vox.rizenhq.com/donation";
