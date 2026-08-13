const pairForm = document.querySelector("#pair-form");
const pairCode = document.querySelector("#pair-code");
const attachButton = document.querySelector("#attach");
const detachButton = document.querySelector("#detach");
const status = document.querySelector("#status");
const error = document.querySelector("#error");

function send(message) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage(message, (response) => {
      const runtimeError = chrome.runtime.lastError;
      if (runtimeError) reject(new Error(runtimeError.message));
      else if (!response?.ok) reject(new Error(response?.error || "Request failed."));
      else resolve(response.value);
    });
  });
}

function render(state) {
  const paired = Boolean(state?.sessionId);
  const attached = Boolean(state?.tabId);
  pairForm.hidden = paired;
  attachButton.disabled = !paired || attached;
  attachButton.hidden = attached;
  detachButton.hidden = !attached;
  status.textContent = attached ? "Connected" : paired ? "Paired" : "Not paired";
}

async function refresh() {
  try {
    render(await send({ type: "status" }));
  } catch (cause) {
    error.textContent = cause.message;
  }
}

pairForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  error.textContent = "";
  try {
    await send({ type: "pair", code: pairCode.value });
    await refresh();
  } catch (cause) {
    error.textContent = cause.message;
  }
});

attachButton.addEventListener("click", async () => {
  error.textContent = "";
  attachButton.disabled = true;
  try {
    await send({ type: "attach" });
    await refresh();
  } catch (cause) {
    error.textContent = cause.message;
    attachButton.disabled = false;
  }
});

detachButton.addEventListener("click", async () => {
  error.textContent = "";
  try {
    await send({ type: "detach" });
    await refresh();
  } catch (cause) {
    error.textContent = cause.message;
  }
});

void refresh();
