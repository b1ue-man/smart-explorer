package app.smartexplorer.android.launcher.search

import java.text.Collator
import java.text.Normalizer
import java.util.Locale
import java.util.concurrent.CopyOnWriteArrayList

/** One hit of an additional source; [source] is the heading of its section (U35-25). */
data class SearchHit(val source: String, val title: String, val subtitle: String? = null, val onClick: () -> Unit)

/**
 * A source of the section „Weitere Quellen“: U35 registers none, U40 registers „Geräte und Räume“. [tokens]
 * are the normalised words of the query (see [LauncherSearch.tokens]).
 */
fun interface SearchSource {
    fun search(tokens: List<String>): List<SearchHit>
}

/**
 * The matching of the app search (U35-25), pure Kotlin. Words are compared without accents and case;
 * an app matches when every query word begins a word of its name (rank 0), else when every query word
 * occurs somewhere in the name (rank 1). There is no background index: the list of apps is in memory
 * and one pass takes milliseconds.
 */
object LauncherSearch {
    /** Additional sources; U40 appends. */
    val sources: MutableList<SearchSource> = CopyOnWriteArrayList()

    private val combining = Regex("\\p{Mn}+")
    private val whitespace = Regex("\\s+")
    private val nonWord = Regex("[^\\p{L}\\p{N}]+")

    /** NFD, combining marks removed, lower case (root locale). */
    fun normalize(text: String): String = Normalizer.normalize(text, Normalizer.Form.NFD).replace(combining, "").lowercase(Locale.ROOT)

    /** The normalised words of [query]; empty for a blank query. */
    fun tokens(query: String): List<String> = normalize(query).trim().split(whitespace).filter { it.isNotEmpty() }

    /** 0: every word starts a word of [label]; 1: every word occurs in it; `null`: no match. [tokens] must not be empty. */
    fun rank(tokens: List<String>, label: String): Int? {
        val normalized = normalize(label)
        val words = normalized.split(nonWord).filter { it.isNotEmpty() }
        if (tokens.all { token -> words.any { it.startsWith(token) } }) return 0
        if (tokens.all { it in normalized }) return 1
        return null
    }

    /**
     * Items of [items] matching [query], ordered by rank, then by [collator] on the label. An empty query
     * returns the whole list in its given order.
     */
    fun <T> filter(
        items: List<T>,
        query: String,
        label: (T) -> String,
        collator: Collator = Collator.getInstance(Locale.getDefault()).apply { strength = Collator.PRIMARY },
    ): List<T> {
        val words = tokens(query)
        if (words.isEmpty()) return items
        return items.mapNotNull { item -> rank(words, label(item))?.let { it to item } }
            .sortedWith(compareBy<Pair<Int, T>> { it.first }.thenComparator { a, b -> collator.compare(label(a.second), label(b.second)) })
            .map { it.second }
    }

    /** Hits of every registered source for [query]; a failing source does not hide the others. */
    fun additional(query: String): List<SearchHit> {
        val words = tokens(query)
        if (words.isEmpty()) return emptyList()
        return sources.flatMap { source -> runCatching { source.search(words) }.getOrDefault(emptyList()) }
    }
}
