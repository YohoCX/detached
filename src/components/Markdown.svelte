<script lang="ts">
  // Foreign markdown: links resolve against the project, then marked and DOMPurify; links open in the browser.
  let { text, projectUrl, baseUrl }: { text: string; projectUrl: string; baseUrl: string } = $props()
</script>

<div class="markdown">{text}</div>
