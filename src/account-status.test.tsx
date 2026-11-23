// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CompanyAccounts } from "./CompanyAccounts";
import type { CompanyAccountEdit, CompanyView } from "./types";

const { native } = vi.hoisted(() => ({ native: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: native }));

let container: HTMLDivElement;
let root: Root;

function button(text: string) {
  const match = Array.from(container.querySelectorAll("button")).find(
    (element) => element.textContent === text,
  );
  if (!match) throw new Error(`Missing button: ${text}`);
  return match;
}

function inventory(status: "active" | "paused"): CompanyView {
  return {
    company_id: "fixture-company",
    companies: ["fixture-company", "fixture-second"],
    signed_in: true,
    accounts: [
      {
        company_id: "fixture-company",
        account_ref: "account-a",
        display_name: "Fixture account",
        provider: "claude",
        revision: status === "active" ? "1" : "2",
        status,
      },
    ],
    bindings: [],
  };
}

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  native.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("company accounts", () => {
  it.each(["transport error", "malformed acknowledgement"])(
    "retries a pause with the same request and revision after a %s",
    async (failure) => {
      let status: "active" | "paused" = "active";
      let outage = false;
      const edits: CompanyAccountEdit[] = [];
      native.mockImplementation(async (command, args) => {
        if (command === "company_accounts") {
          if (outage) throw new Error("Company status is unavailable");
          return inventory(status);
        }
        if (command !== "company_set_status") throw new Error(command);
        edits.push(args.edit);
        status = "paused";
        if (edits.length === 1) {
          outage = true;
          if (failure === "transport error") {
            throw "The save result is unavailable. Retry the same change to check it";
          }
          return { outcome: "unexpected" };
        }
        outage = false;
        return { outcome: "unchanged" };
      });
      await act(async () =>
        root.render(<CompanyAccounts profiles={[]} desktopActivePaths={[]} />),
      );
      await act(async () => button("Pause").click());
      expect(container.textContent).not.toContain("Fixture account");
      await act(async () => button("Retry save").click());
      expect(container.textContent).toContain("Paused");
      expect(edits[0]).toMatchObject({
        expected_revision: "1",
        desired_status: "paused",
      });
      expect(edits[1]).toEqual(edits[0]);
      expect(container.textContent).not.toContain("Retry save");
    },
  );

  it.each([
    ["stale_revision", "The account changed."],
    ["request_id_conflict", "This request ID was already used"],
  ])("reports %s and reloads the current status", async (outcome, message) => {
    native
      .mockResolvedValueOnce(inventory("active"))
      .mockResolvedValueOnce({ outcome })
      .mockResolvedValueOnce(inventory("paused"));
    await act(async () =>
      root.render(<CompanyAccounts profiles={[]} desktopActivePaths={[]} />),
    );
    await act(async () => button("Pause").click());
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(
      message,
    );
    expect(container.textContent).toContain("Paused");
    expect(container.textContent).not.toContain("Retry save");
    expect(button("Reactivate").disabled).toBe(false);
  });

  it.each([true, false])(
    "compares native path variants for credential presence (Windows: %s)",
    async (windowsPaths) => {
      const snapshot = inventory("active");
      snapshot.accounts[0].provider = "codex";
      snapshot.bindings = [
        { provider: "codex", account_ref: "account-a", path: "work/profile" },
      ];
      native.mockResolvedValueOnce(snapshot);
      await act(async () =>
        root.render(
          <CompanyAccounts
            profiles={[
              {
                provider: "codex",
                name: "Profile",
                email: "",
                family: "Work",
                aliases: [],
                profilePath: "Work/Profile",
                status: "",
                configured: true,
                credentialPresent: true,
                active: false,
                source: "manifest",
              },
            ]}
            desktopActivePaths={["WORK\\PROFILE"]}
            windowsPaths={windowsPaths}
          />,
        ),
      );
      expect(container.textContent).toContain(
        windowsPaths
          ? "Local credential present · Selected on this device"
          : "No local credential",
      );
    },
  );

  it("drops a late inventory response after the operator switches companies", async () => {
    let resolveOld: (value: CompanyView) => void = () => undefined;
    let oldRefresh = 0;
    let second = false;
    native.mockImplementation(async (command, args) => {
      if (command === "company_select") {
        expect(args).toEqual({ companyId: "fixture-second" });
        second = true;
        return;
      }
      if (second)
        return {
          ...inventory("active"),
          company_id: "fixture-second",
          signed_in: false,
          accounts: [],
        };
      if (oldRefresh++ === 0) return inventory("active");
      return new Promise<CompanyView>((resolve) => {
        resolveOld = resolve;
      });
    });
    await act(async () =>
      root.render(<CompanyAccounts profiles={[]} desktopActivePaths={[]} />),
    );
    await act(async () => button("Refresh status").click());
    const select = container.querySelector("select");
    if (!select) throw new Error("Missing company selector");
    await act(async () => {
      select.value = "fixture-second";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => resolveOld(inventory("paused")));
    expect(container.textContent).toContain(
      "Company accounts · fixture-second",
    );
    expect(container.textContent).not.toContain("Fixture account");
  });

  it("clears the old inventory when the authority stops answering", async () => {
    native.mockResolvedValueOnce(inventory("active"));
    native.mockRejectedValueOnce(new Error("Company status is unavailable"));
    await act(async () =>
      root.render(<CompanyAccounts profiles={[]} desktopActivePaths={[]} />),
    );
    expect(container.textContent).toContain("Fixture account");
    await act(async () => button("Refresh status").click());
    expect(container.textContent).not.toContain("Fixture account");
    expect(container.textContent).toContain("Company status is unavailable");
  });
});
