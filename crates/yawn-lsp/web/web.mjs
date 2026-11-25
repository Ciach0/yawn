// HTML and CSS intelligence for yawn-lsp. One JSON request per stdin line, one JSON answer per
// stdout line: {id, op, text, line, character} -> {id, result}. `text` is a virtual document in which
// everything that is not HTML (or not CSS) has been blanked out, so positions equal the .rsx positions.
import { createInterface } from 'node:readline';
import { getLanguageService as html } from 'vscode-html-languageservice';
import { getCSSLanguageService as css } from 'vscode-css-languageservice';
import { TextDocument } from 'vscode-languageserver-textdocument';

const htmlLs = html();
const cssLs = css();
const markdown = { documentation: ['markdown', 'plaintext'], contentFormat: ['markdown', 'plaintext'] };

function run({ op, text, line = 0, character = 0 }) {
  const position = { line, character };
  switch (op) {
    case 'htmlComplete': {
      const doc = TextDocument.create('file:///v.html', 'html', 0, text);
      return htmlLs.doComplete(doc, position, htmlLs.parseHTMLDocument(doc)).items;
    }
    case 'htmlHover': {
      const doc = TextDocument.create('file:///v.html', 'html', 0, text);
      return htmlLs.doHover(doc, position, htmlLs.parseHTMLDocument(doc));
    }
    case 'cssComplete': {
      const doc = TextDocument.create('file:///v.css', 'css', 0, text);
      return cssLs.doComplete(doc, position, cssLs.parseStylesheet(doc)).items;
    }
    case 'cssHover': {
      const doc = TextDocument.create('file:///v.css', 'css', 0, text);
      return cssLs.doHover(doc, position, cssLs.parseStylesheet(doc));
    }
    case 'cssDiagnostics': {
      const doc = TextDocument.create('file:///v.css', 'css', 0, text);
      return cssLs.doValidation(doc, cssLs.parseStylesheet(doc));
    }
    default:
      return null;
  }
}

cssLs.configure({ validate: true });
createInterface({ input: process.stdin }).on('line', (l) => {
  let id = null;
  try {
    const req = JSON.parse(l);
    id = req.id;
    process.stdout.write(JSON.stringify({ id, result: run(req) ?? null }) + '\n');
  } catch (e) {
    process.stdout.write(JSON.stringify({ id, error: String(e) }) + '\n');
  }
});
