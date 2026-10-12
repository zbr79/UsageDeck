import { toast } from "react-hot-toast";

const MAX_TOASTS = 3;
const toastQueue: string[] = [];

// Absolute kill switch (must be > duration)
const HARD_CLOSE_MS = 2500;

function pushToast(id: string) {
  toastQueue.push(id);

  if (toastQueue.length > MAX_TOASTS) {
    const oldest = toastQueue.shift();
    if (oldest) toast.dismiss(oldest);
  }
}

function showToast(create: () => string) {
  const id = create();
  pushToast(id);

  // Absolute safety: nothing can live forever
  setTimeout(() => {
    toast.dismiss(id);
  }, HARD_CLOSE_MS);
}

/* ======================
   Exported helpers
====================== */

export const toastSuccess = (message: string) => {
  showToast(() => toast.success(message));
};

export const toastError = (message: string) => {
  showToast(() => toast.error(message));
};
