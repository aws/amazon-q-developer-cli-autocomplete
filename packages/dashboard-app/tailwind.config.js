import * as tailwindcssAnimate from "tailwindcss-animate";

/** @type {import('tailwindcss').Config} */
export default {
  darkMode: ["media"],
  content: [
    "./pages/**/*.{ts,tsx}",
    "./components/**/*.{ts,tsx}",
    "./app/**/*.{ts,tsx}",
    "./src/**/*.{ts,tsx}",
  ],
  fontSize: {
    sm: "0.66rem",
    base: "0.75rem",
    md: "0.875rem",
    lg: "1rem",
    xl: "1.5rem",
    "2xl": "2rem",
    "3xl": "3rem",
  },
  theme: {
    container: {
      center: true,
      padding: "2rem",
      screens: {
        "2xl": "1400px",
      },
    },
    extend: {
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        cyan: {
          50: "#eef7ff",
          100: "#d9ebff",
          200: "#bcddff",
          300: "#8ec8ff",
          400: "#59a9ff",
          500: "#3e8dff",
          600: "#1b65f5",
          700: "#1450e1",
          800: "#1741b6",
          900: "#193a8f",
          950: "#142557",
        },
        dusk: {
          50: "#f4f0ff",
          100: "#ebe0ff",
          200: "#d8c5ff",
          300: "#bc9dff",
          400: "#9c6bff",
          500: "#8700ff",
          600: "#7700e6",
          700: "#6600cc",
          800: "#5500b3",
          900: "#440099",
          950: "#330066",
        },
        "dusk-dark": {
          50: "#faf8ff",
          100: "#f3edff",
          200: "#e9ddff",
          300: "#d9c2ff",
          400: "#c19aff",
          500: "#a970ff",
          600: "#9347ff",
          700: "#7c2eff",
          800: "#6b1fff",
          900: "#5a1ae6",
          950: "#3d0fb3",
        },
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      keyframes: {
        "accordion-down": {
          from: { height: 0 },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: 0 },
        },
        shimmer: {
          "100%": {
            transform: "translateX(100%)",
          },
        },
        shine: {
          "0%": {
            filter: "brightness(100%)",
          },
          "50%": {
            filter: "brightness(150%)",
          },
          "100%": {
            filter: "brightness(100%)",
          },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
        shimmer: "shimmer 4s ease-in-out infinite",
        shine: "shine 4s ease-in-out infinite",
      },
      fontFamily: {
        ember: ["Ember", "sans-serif"],
      },
    },
  },
  plugins: [tailwindcssAnimate],
};
