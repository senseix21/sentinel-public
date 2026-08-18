import { esc, icon } from "../formatting.js";

export function absent(title, why, needs) {
  return `<div class="absent">
      <h4>${icon("alert")}${esc(title)}</h4>
      <p>${why}</p>
      ${needs && needs.length ? `<ul>${needs.map((need) => `<li>${need}</li>`).join("")}</ul>` : ""}
    </div>`;
}
