"use client";

import { Toaster } from "react-hot-toast";

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
          color: "#1f2937",
          background: "#f9fafb",
        },

        success: {
          style: {
            background: "#ecfdf5",
            borderLeft: "5px solid #22c55e",
          },
        },

        error: {
          style: {
            background: "#fef2f2",
            borderLeft: "5px solid #ef4444",
          },
        },

        loading: {
          style: {
            background: "#eff6ff",
            borderLeft: "5px solid #3b82f6",
          },
        },
      }}
    />
  );
}
