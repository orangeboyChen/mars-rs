package io.github.orangeboychen.marsrs

import kotlinx.cinterop.ByteVar
import kotlinx.cinterop.CPointer
import kotlinx.cinterop.CPointerVar
import kotlinx.cinterop.ExperimentalForeignApi
import kotlinx.cinterop.NativePtr
import kotlinx.cinterop.UByteVar
import kotlinx.cinterop.allocArray
import kotlinx.cinterop.get
import kotlinx.cinterop.nativeHeap
import kotlinx.cinterop.rawValue
import kotlinx.cinterop.set

/**
 * The memory of one answer, which is the one thing the C ABI asks a caller to
 * keep: an answer is read *after* the ask that wrote it has returned, so the
 * bytes and the strings it points at cannot be the ones of a scope that ends
 * with the ask.
 *
 * So they are copies on the heap, and this is what keeps them: every allocation
 * a box makes is remembered here by its [NativePtr], and [clear] frees them all —
 * which the box does when the *next* question is asked, by which time the port
 * has read the answer of the one before.
 */
@OptIn(ExperimentalForeignApi::class)
internal class Held {
    /** Every copy this made, as the address [clear] frees it by. */
    private val allocations = mutableListOf<NativePtr>()

    /** The addresses of a resolve, as the `char**` a count goes with. */
    fun strings(values: List<String>): CPointer<CPointerVar<ByteVar>> {
        val array = keep(nativeHeap.allocArray<CPointerVar<ByteVar>>(values.size))
        values.forEachIndexed { index, value -> array[index] = cstring(value) }
        return array
    }

    /** What a task sends, or the hash an answer is judged against. */
    fun bytes(values: ByteArray): CPointer<UByteVar> {
        val array = keep(nativeHeap.allocArray<UByteVar>(values.size))
        values.forEachIndexed { index, byte -> array[index] = byte.toUByte() }
        return array
    }

    /** Frees every copy this made, which is what the next question is asked over. */
    fun clear() {
        allocations.forEach { nativeHeap.free(it) }
        allocations.clear()
    }

    /** One string, as a NUL-terminated copy the port reads after the ask. */
    private fun cstring(value: String): CPointer<ByteVar> {
        val encoded = value.encodeToByteArray()
        val pointer = keep(nativeHeap.allocArray<ByteVar>(encoded.size + 1))
        encoded.forEachIndexed { index, byte -> pointer[index] = byte }
        pointer[encoded.size] = 0
        return pointer
    }

    private fun <T : CPointer<*>> keep(allocation: T): T {
        allocations.add(allocation.rawValue)
        return allocation
    }
}
