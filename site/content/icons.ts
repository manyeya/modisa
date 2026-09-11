import solar from "@iconify-json/solar/icons.json";
// Solar icons by 480 Design, CC BY 4.0, distributed by Iconify.
export function icon(name: keyof typeof solar.icons) {
  const data = solar.icons[name];
  return `<svg class="icon" viewBox="0 0 24 24" width="20" height="20" aria-hidden="true" focusable="false">${data.body}</svg>`;
}
