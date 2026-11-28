package app.smartexplorer.android.ui.viewer

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import app.smartexplorer.android.api.MdDocumentDto
import app.smartexplorer.android.api.StyledLineDto
import app.smartexplorer.android.api.StyledSpanDto
import app.smartexplorer.android.api.TextLines
import app.smartexplorer.android.core.Core
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Text view (api.md §4.15): spans → AnnotatedString, "Gehe zu Zeile", Markdown links, answer shapes. */
class TextViewTaskTest {
    @Test
    fun textview_task_spans_become_annotated_string_styles() {
        // "größe 😀 x": the emoji takes two UTF-16 units, as the core counts them.
        val line = StyledLineDto(
            number = 4,
            text = "größe 😀 x",
            spans = listOf(
                StyledSpanDto(start = 0, end = 5, fg = 0xFF112233, bold = true),
                StyledSpanDto(start = 5, end = 9, fg = 0x80FF0000, bg = 0xFF00FF00, italic = true, underline = true),
                StyledSpanDto(start = 9, end = 10, fg = 0xFF000000),
            ),
        )
        assertEquals(10, line.text.length)
        val text = styledText(line)
        assertEquals(line.text, text.text)
        val styles = text.spanStyles
        assertEquals(3, styles.size)
        assertEquals("größe", text.text.substring(styles[0].start, styles[0].end))
        assertEquals(Color(0xFF112233), styles[0].item.color)
        assertEquals(FontWeight.Bold, styles[0].item.fontWeight)
        assertEquals(" 😀 ", text.text.substring(styles[1].start, styles[1].end))
        assertEquals(Color(0x80FF0000), styles[1].item.color)
        assertEquals(Color(0xFF00FF00), styles[1].item.background)
        assertEquals(FontStyle.Italic, styles[1].item.fontStyle)
        assertEquals(TextDecoration.Underline, styles[1].item.textDecoration)
        assertEquals(Color.Unspecified, styles[2].item.background)
        // A find match is laid over the spans.
        val mark = SpanStyle(background = Color.Yellow)
        val marked = styledText(line, 6 until 8, mark)
        assertEquals(4, marked.spanStyles.size)
        assertEquals("😀", marked.text.substring(marked.spanStyles[3].start, marked.spanStyles[3].end))
    }

    @Test
    fun textview_task_spans_outside_the_text_are_clamped() {
        val line = StyledLineDto(text = "abc", spans = listOf(StyledSpanDto(start = 2, end = 50, fg = 0xFF000000), StyledSpanDto(start = 7, end = 9, fg = 0xFF000000)))
        val text = styledText(line, 1 until 99, SpanStyle(background = Color.Yellow))
        assertEquals(listOf(2 to 3, 1 to 3), text.spanStyles.map { it.start to it.end })
        // No match range and no spans: the plain text.
        assertTrue(styledText(StyledLineDto(text = "")).spanStyles.isEmpty())
        assertEquals(Color(0xFF00FF00), argbColor(0xFF00FF00))
    }

    @Test
    fun textview_task_go_to_line_bounds() {
        assertEquals(GoToLine.Line(11), resolveGoToLine("12", known = 100, complete = true))
        assertEquals(GoToLine.Line(0), resolveGoToLine("1", known = 100, complete = true))
        assertEquals(GoToLine.Line(99), resolveGoToLine("100", known = 100, complete = true))
        assertEquals(GoToLine.Line(4), resolveGoToLine(" 5 ", known = 10, complete = false))
        assertEquals(GoToLine.Invalid("Die Datei hat 100 Zeilen."), resolveGoToLine("101", known = 100, complete = true))
        val reading = resolveGoToLine("101", known = 100, complete = false)
        assertTrue(reading is GoToLine.Invalid && reading.message.contains("noch gelesen"))
        assertTrue(resolveGoToLine("0", known = 100, complete = true) is GoToLine.Invalid)
        assertTrue(resolveGoToLine("-3", known = 100, complete = true) is GoToLine.Invalid)
        assertTrue(resolveGoToLine("zwölf", known = 100, complete = true) is GoToLine.Invalid)
        assertTrue(resolveGoToLine("", known = 100, complete = true) is GoToLine.Invalid)
        assertTrue(resolveGoToLine("5", known = 0, complete = false) is GoToLine.Invalid)
    }

    @Test
    fun textview_task_markdown_links_resolve_against_the_document() {
        val local = "/storage/emulated/0/Projekt/docs/LIESMICH.md"
        assertEquals(LinkTarget.File("/storage/emulated/0/Projekt/docs/andere.md", "andere.md"), resolveLink("", local, "andere.md"))
        assertEquals(LinkTarget.File("/storage/emulated/0/Projekt/docs/teil/b.md", "b.md"), resolveLink("", local, "./teil/b.md#abschnitt"))
        assertEquals(LinkTarget.File("/storage/emulated/0/Projekt/bilder/a b.png", "a b.png"), resolveLink("", local, "../bilder/a%20b.png"))
        assertEquals(LinkTarget.File("/storage/emulated/0/Projekt/docs/C++.md", "C++.md"), resolveLink("", local, "C++.md?raw=1"))
        // "/…" is the root of the same connection; ".." never climbs above it.
        val remote = "/home/anna/notizen/x.md"
        assertEquals(LinkTarget.File("sftp://anna@server:22/etc/hosts", "hosts"), resolveLink("sftp://anna@server:22", remote, "/etc/hosts"))
        assertEquals(LinkTarget.File("sftp://anna@server:22/y.md", "y.md"), resolveLink("sftp://anna@server:22", remote, "../../../../../y.md"))
        assertEquals(
            LinkTarget.File("share://direct/geraet-1/Dokumente/z.md", "z.md"),
            resolveLink("share://direct/geraet-1", "/Dokumente/notiz.md", "z.md"),
        )
        assertEquals(LinkTarget.External("https://example.org/a?b=1#c"), resolveLink("", local, "https://example.org/a?b=1#c"))
        assertEquals(LinkTarget.External("mailto:anna@example.org"), resolveLink("", local, "mailto:anna@example.org"))
        assertEquals(LinkTarget.None, resolveLink("", local, "#abschnitt"))
        assertEquals(LinkTarget.None, resolveLink("", local, "javascript:alert(1)"))
        assertEquals(LinkTarget.None, resolveLink("", local, "  "))
        assertEquals(LinkTarget.None, resolveLink("", "/a.md", ".."))
    }

    @Test
    fun textview_task_bridge_answers_decode() {
        val lines = Core.json.decodeFromString(
            TextLines.serializer(),
            """{"first":10,"language":"Rust","known":120,"complete":false,
               "colors":{"theme":"OneHalfDark","background":4280295989,"foreground":4292138196,"selection":1088479487,"lineHighlight":4280295989,"gutter":4286611584},
               "lines":[{"number":10,"text":"fn ä()","spans":[{"start":0,"end":2,"fg":4291653755,"bg":null,"bold":false,"italic":false,"underline":false}]}]}""",
        )
        assertEquals(10L, lines.lines[0].number)
        assertNull(lines.lines[0].spans[0].bg)
        assertEquals(Color(4280295989L.toInt()), argbColor(lines.colors.background))
        val markdown = Core.json.decodeFromString(
            MdDocumentDto.serializer(),
            """{"blocks":[
                {"type":"heading","level":2,"runs":[{"text":"Titel","kind":{"type":"text"},"strong":false,"emphasis":false,"strikethrough":false,"link":null}]},
                {"type":"list","start":3,"items":[{"task":true,"blocks":[{"type":"paragraph","runs":[{"text":"x","kind":{"type":"code"},"strong":true,"emphasis":false,"strikethrough":false,"link":"b.md"}]}]}]},
                {"type":"code","language":"rs","highlightedAs":"Rust","lines":[{"text":"let ü","spans":[{"start":0,"end":5,"fg":4278190080}]}]},
                {"type":"table","alignments":["left","right"],"header":[[{"text":"A","kind":{"type":"text"}}],[]],"rows":[[[],[{"text":"2","kind":{"type":"text"}}]]]},
                {"type":"quote","kind":"warning","blocks":[]},
                {"type":"image","url":"bild.png","title":"","alt":"Bild"},
                {"type":"künftig","text":"neu"}
            ]}""",
        )
        val blocks = markdown.blocks
        assertEquals(listOf("heading", "list", "code", "table", "quote", "image", "künftig"), blocks.map { it.type })
        assertEquals(3L, blocks[1].start)
        assertEquals(true, blocks[1].items[0].task)
        assertEquals("b.md", blocks[1].items[0].blocks[0].runs[0].link)
        assertEquals("code", blocks[1].items[0].blocks[0].runs[0].kind.type)
        assertEquals("Rust", blocks[2].highlightedAs)
        assertEquals(listOf("left", "right"), blocks[3].alignments)
        assertEquals("2", blocks[3].rows[0][1][0].text)
        assertEquals("warning", blocks[4].kind)
        assertEquals("Bild", blocks[5].alt)
    }
}
