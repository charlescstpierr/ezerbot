import assert from "node:assert/strict";
import { test } from "node:test";
import { formatMeetingDuration } from "./meeting-presentation.js";

test("meeting durations remain readable from one second through the ten-minute cap", () => {
  for (const [milliseconds, expected] of [[1000,"1 s"],[59999,"1 min"],[60000,"1 min"],[65000,"1 min 05 s"],[135000,"2 min 15 s"],[200000,"3 min 20 s"],[245000,"4 min 05 s"],[600000,"10 min"]]) {
    assert.equal(formatMeetingDuration(milliseconds),expected);
  }
});
test("invalid duration metadata never becomes NaN or an infinite duration", () => {
  for (const input of [undefined,null,-1,NaN,Infinity,"invalide"]) assert.equal(formatMeetingDuration(input),"0 s");
});
