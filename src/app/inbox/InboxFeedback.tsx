import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

// Reports are untrusted: only absolute HTTP(S) links may be opened; images and HTML are inert.
const components: Components = {
  a: ({ children, href }) => {
    if (href && /^https?:\/\//i.test(href)) {
      try {
        const url = new URL(href);
        if (url.protocol === "https:" || url.protocol === "http:") return <a href={url.href} target="_blank" rel="noopener noreferrer">{children}</a>;
      } catch { /* Render invalid URLs as text. */ }
    }
    return <span>{children}</span>;
  },
  img: ({ alt }) => <span>{alt || "image"}</span>,
  pre: ({ children }) => <pre tabIndex={0}>{children}</pre>,
  table: ({ children }) => <table tabIndex={0}>{children}</table>,
};
const plugins = [remarkGfm];

export function InboxFeedback({ text }: { text: string }) {
  return <div className="inbox-feedback"><ReactMarkdown skipHtml remarkPlugins={plugins} components={components}>{text}</ReactMarkdown></div>;
}
