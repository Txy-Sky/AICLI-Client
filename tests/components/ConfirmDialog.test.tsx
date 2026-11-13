import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "@/components/ConfirmDialog";

describe("ConfirmDialog", () => {
  it("prevents cancel and confirm actions while pending", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();

    render(
      <ConfirmDialog
        isOpen
        title="Confirm write"
        message="Please wait"
        pending
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );

    const cancelButton = screen.getByRole("button", { name: "common.cancel" });
    const confirmButton = screen.getByRole("button", {
      name: "common.confirm",
    });
    expect(cancelButton).toBeDisabled();
    expect(confirmButton).toBeDisabled();

    fireEvent.click(cancelButton);
    fireEvent.click(confirmButton);
    expect(onCancel).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("shows the conclusion with its optional supplements and passes the checkbox back", () => {
    const onConfirm = vi.fn();
    render(
      <ConfirmDialog
        isOpen
        title="Rebuild"
        message="Clears and imports again."
        assurance="Backed up first."
        note="Deleted logs are lost."
        code="hermes dashboard"
        hint="Run pip first."
        checkboxLabel="Also migrate"
        checkboxHint="Restorable."
        onConfirm={onConfirm}
        onCancel={() => undefined}
      />,
    );
    const dialog = screen.getByRole("dialog");
    // 结论仍是对话框的描述，供读屏
    expect(dialog).toHaveAccessibleDescription("Clears and imports again.");
    expect(screen.getByTestId("confirm-assurance")).toHaveTextContent(
      "Backed up first.",
    );
    expect(screen.getByTestId("confirm-note")).toHaveTextContent(
      "Deleted logs are lost.",
    );
    expect(screen.getByTestId("confirm-code")).toHaveTextContent(
      "hermes dashboard",
    );
    expect(screen.getByText("Run pip first.")).toBeInTheDocument();
    expect(screen.getByText("Restorable.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "common.confirm" }));
    expect(onConfirm).toHaveBeenCalledWith(true);
  });

  it("renders only the conclusion when no supplement is given", () => {
    render(
      <ConfirmDialog
        isOpen
        title="Leave"
        message="Changes are lost."
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />,
    );
    expect(screen.queryByTestId("confirm-assurance")).not.toBeInTheDocument();
    expect(screen.queryByTestId("confirm-note")).not.toBeInTheDocument();
    expect(screen.queryByTestId("confirm-code")).not.toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  });
});
