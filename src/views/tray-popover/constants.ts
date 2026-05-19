import { type Period } from "@/lib/ipc";

export const PERIOD_OPTIONS: { value: Period; label: string }[] = [
  { value: "last_session", label: "Last session" },
  { value: "day", label: "Today" },
  { value: "week", label: "Week" },
  { value: "month", label: "Month" },
  { value: "year", label: "Year" },
];
