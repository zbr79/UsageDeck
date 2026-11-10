"use client";

import { Toaster } from "react-hot-toast";
import { TOAST_PALETTE } from "@/lib/theme";

export default function ToastProvider() {
  return (
    <Toaster
      position="top-center"
      toastOptions={{
        duration: 2500,

        style: {
          borderRadius: "10px",
          padding: "10px 14px",
          fontSize: "14px",
          boxShadow: "0 6px 20px rgba(0,0,0,0.08)",
          color: TOAST_PALETTE.baseText,
          background: TOAST_PALETTE.baseBg,
        },

        success: {
          style: {
            background: TOAST_PALETTE.successBg,
            borderLeft: `5px solid ${TOAST_PALETTE.successBorder}`,
          },
        },

        error: {
          style: {
            background: TOAST_PALETTE.errorBg,
            borderLeft: `5px solid ${TOAST_PALETTE.errorBorder}`,
          },
        },

        loading: {
          style: {
            background: TOAST_PALETTE.loadingBg,
            borderLeft: `5px solid ${TOAST_PALETTE.loadingBorder}`,
          },
        },
      }}
    />
  );
}
