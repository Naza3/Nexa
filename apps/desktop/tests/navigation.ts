import { fireEvent, screen } from "@testing-library/react";

export function openGroup(title: string) {
  const label = screen.getByText(title, { selector: ".details-group > summary strong" });
  const details = label.closest("details")!;
  if (!details.open) fireEvent.click(label);
}
export function openSettingsGroups() {
  for (const summary of document.querySelectorAll<HTMLDetailsElement>(".workspace-pages > .details-group")) {
    if (!summary.open) fireEvent.click(summary.querySelector("summary")!);
  }
}
export async function openModelDetails(name: string) {
  const existing = screen.queryByRole("heading", { name, level: 1 });
  if (!existing) fireEvent.click(await screen.findByRole("button", { name: `查看 ${name} 的详情` }));
  return screen.getByRole("heading", { name, level: 1 }).closest("article")!;
}
export function reveal(element: HTMLElement) {
  let parent = element.parentElement;
  while (parent) {
    if (parent instanceof HTMLDetailsElement && !parent.open) fireEvent.click(parent.querySelector("summary")!);
    parent = parent.parentElement;
  }
  return element;
}
