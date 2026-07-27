import assert from "node:assert/strict";
import test from "node:test";
import { runtimeInterruptionFromSdk } from "../src/sdk-engine.ts";
import { REQUEST_CLARIFICATION_TOOL } from "../src/types.ts";

test("maps request_clarification approval pauses to structured clarification interruptions", () => {
  const interruption = runtimeInterruptionFromSdk({
    id: "clarify-1",
    name: "request_clarification",
    arguments: JSON.stringify({
      question: "Which project should I update?",
      choices: ["June", "Accounts"],
    }),
  });
  assert.deepEqual(interruption, {
    id: "clarify-1",
    kind: "clarification",
    toolName: "request_clarification",
    arguments: {
      question: "Which project should I update?",
      choices: ["June", "Accounts"],
    },
    question: "Which project should I update?",
    choices: ["June", "Accounts"],
  });
});

test("prefers canonical raw interruption identities over legacy top-level identities", () => {
  const interruption = runtimeInterruptionFromSdk({
    id: "legacy-id",
    callId: "legacy-call-id",
    rawItem: {
      callId: "raw-call-id",
      id: "raw-id",
      providerData: { itemId: "provider-item-id", id: "provider-id" },
    },
  });

  assert.equal(interruption.id, "raw-call-id");
});

test("uses provider data identities and snake_case aliases", () => {
  assert.equal(
    runtimeInterruptionFromSdk({
      rawItem: { providerData: { itemId: "provider-item-id" } },
    }).id,
    "provider-item-id",
  );
  assert.equal(
    runtimeInterruptionFromSdk({
      rawItem: { call_id: "snake-call-id", provider_data: { item_id: "snake-item-id" } },
    }).id,
    "snake-call-id",
  );
});

test("materializes one stable identity for an identifier-less SDK interruption", () => {
  const sdkInterruption: {
    rawItem: { providerData?: { itemId?: string } };
  } = { rawItem: {} };

  const first = runtimeInterruptionFromSdk(sdkInterruption).id;
  const second = runtimeInterruptionFromSdk(sdkInterruption).id;

  assert.notEqual(first, "unknown-interruption");
  assert.equal(second, first);
  assert.equal(sdkInterruption.rawItem.providerData?.itemId, first);
});

test("generates distinct identities for separate identifier-less SDK interruptions", () => {
  const first = runtimeInterruptionFromSdk({ rawItem: {} }).id;
  const second = runtimeInterruptionFromSdk({ rawItem: {} }).id;

  assert.notEqual(first, second);
});

test("the built-in clarification tool always pauses for a user answer", () => {
  assert.equal(REQUEST_CLARIFICATION_TOOL.name, "request_clarification");
  assert.equal(REQUEST_CLARIFICATION_TOOL.requiresApproval, true);
  assert.deepEqual(REQUEST_CLARIFICATION_TOOL.parameters.required, ["question"]);
});
