import MarkdownIt from "markdown-it";

const planMarkdown = new MarkdownIt({
  html: false,
  breaks: true,
  linkify: false,
});

// Plan text comes from Claude Code. Only safe destinations are allowed, links
// are rendered as plain labels so the panel never navigates away, and images
// fall back to their alt text instead of loading external resources.
planMarkdown.validateLink = (url: string) =>
  /^(?:https?:|mailto:|#|$)/i.test(url);

planMarkdown.renderer.rules.link_open = () => "";
planMarkdown.renderer.rules.link_close = () => "";
planMarkdown.renderer.rules.image = (tokens, index) =>
  planMarkdown.utils.escapeHtml(tokens[index].content);

export const renderMarkdown = (markdown: string): string =>
  planMarkdown.render(markdown);

// Keep the plan-specific name for existing callers while sharing the same
// safe Markdown configuration with session transcript entries.
export const renderPlanMarkdown = renderMarkdown;
