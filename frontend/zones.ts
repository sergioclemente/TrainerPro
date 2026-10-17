// Coggan power zones as the UI colours them. Pure, so the mini HUD's view
// model can be tested without React or Tauri.

/** Zone colour for a target or reading expressed as a percentage of FTP. */
export function zoneColor(pct: number): string {
  if (pct < 55) return "#6e7681"; // Z1 recovery
  if (pct <= 75) return "#388bfd"; // Z2 endurance
  if (pct <= 90) return "#3fb950"; // Z3 tempo
  if (pct <= 105) return "#d29922"; // Z4 threshold
  if (pct <= 120) return "#f0883e"; // Z5 vo2
  if (pct <= 150) return "#f85149"; // Z6 anaerobic
  return "#bc8cff"; // Z7 neuromuscular
}
