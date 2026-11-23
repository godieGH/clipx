package com.godiegh.clipx.ui

import androidx.compose.foundation.Canvas
import androidx.compose.ui.geometry.Offset
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.godiegh.clipx.FileTransferUi
import com.godiegh.clipx.transferKey
import com.godiegh.clipx.ui.theme.LocalClipxPalette
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.ui.graphics.Color
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeJoin

/**
 * Downloads and uploads in one list, one row per transfer. A transfer is a file
 * going to, or coming from, one device, so an offer sent to two devices is two
 * rows. Tapping the ring pauses or resumes; the word at the right cancels (or
 * clears a finished row from the list).
 */
@Composable
fun TransfersScreen(
    transfers: List<FileTransferUi>,
    onBack: () -> Unit,
    onToggle: (FileTransferUi) -> Unit,
    onCancel: (FileTransferUi) -> Unit,
    onClear: (FileTransferUi) -> Unit,
    onClearFinished: () -> Unit,
) {
    val palette = LocalClipxPalette.current
    val listState = rememberLazyListState()
    val elevated by remember { derivedStateOf { listState.canScrollBackward } }
    // Running and paused transfers first, finished ones below; each group keeps
    // the order the transfers appeared in.
    val rows = remember(transfers) { transfers.sortedBy { if (it.isFinished()) 1 else 0 } }

    Column(modifier = Modifier.fillMaxSize()) {
        ScreenHeader(
            "Transfers",
            onBack,
            elevated = elevated,
            rightContent = {
                if (transfers.any { it.isFinished() }) {
                    TextButton(onClick = onClearFinished) { Text("Clear finished") }
                }
            },
        )
        if (rows.isEmpty()) {
            Text(
                "No transfers right now. Files you send or download show up here.",
                color = palette.mutedText,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 14.dp),
            )
        } else {
            LazyColumn(
                state = listState,
                modifier = Modifier.weight(1f),
                contentPadding = PaddingValues(top = 2.dp, bottom = 18.dp),
            ) {
                items(rows, key = { transferKey(it.deviceId, it.fileId) }) { row ->
                    TransferItem(row, onToggle, onCancel, onClear)
                }
            }
        }
    }
}

@Composable
private fun TransferItem(
    row: FileTransferUi,
    onToggle: (FileTransferUi) -> Unit,
    onCancel: (FileTransferUi) -> Unit,
    onClear: (FileTransferUi) -> Unit,
) {
    val palette = LocalClipxPalette.current
    // An interrupted row is waiting for the other device; cancelling it is not
    // possible, so the word clears it from the list instead.
    val clearsRow = row.isFinished() || row.state == "interrupted"
    GlassCard(modifier = Modifier.padding(horizontal = 18.dp, vertical = 4.dp)) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TransferRing(row, onClick = { onToggle(row) })
            Spacer(Modifier.width(10.dp))
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    row.fileName.ifBlank { "File" },
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    transferSubtitle(row),
                    color = palette.mutedText,
                    style = MaterialTheme.typography.labelSmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 3.dp),
                )
            }
            if (row.state != "saving") {
                TextButton(onClick = { if (clearsRow) onClear(row) else onCancel(row) }) {
                    Text(if (clearsRow) "Clear" else "Cancel")
                }
            }
        }
    }
}

/**
 * Circular progress with the percent in the middle. Tapping it pauses a running
 * transfer and resumes a paused one. When the other device is an older build
 * that cannot pause, the ring is dimmed instead of hidden: tapping it still
 * explains why, and Cancel keeps working.
 */
@Composable
private fun TransferRing(row: FileTransferUi, onClick: () -> Unit) {
    val palette = LocalClipxPalette.current
    val running = row.state == "requesting" || row.state == "receiving" || row.state == "sending"
    val tappable = running || row.state == "paused" || row.state == "interrupted"
    val dimmed = running && !row.canPause
    val percent = if (row.total > 0L) (row.done.toFloat() / row.total.toFloat()).coerceIn(0f, 1f) else 0f
    val arcColor = when (row.state) {
        "paused", "interrupted" -> palette.mutedText
        "failed", "expired" -> MaterialTheme.colorScheme.error
        else -> MaterialTheme.colorScheme.primary
    }
    val trackColor = palette.mutedText.copy(alpha = 0.25f)
    val label = when (row.state) {
        "complete" -> "✓"
        "failed", "expired" -> "!"
        "cancelled" -> "–"
        else -> "${(percent * 100f).toInt()}%"
    }
    val sending = row.direction == "send"
    val kind = if (sending) "upload" else "download"
    val description = when {
        row.state == "paused" || row.state == "interrupted" -> "Resume $kind"
        running -> "Pause $kind"
        else -> "${kind.replaceFirstChar { it.uppercase() }} ${row.state}"
    }
    Box(
        modifier = Modifier
            .size(52.dp)
            .alpha(if (dimmed) 0.5f else 1f)
            .clip(CircleShape)
            .then(if (tappable) Modifier.clickable(onClick = onClick) else Modifier)
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) {
        if (running) FlowArrows(sending, arcColor)
        Canvas(modifier = Modifier
            .fillMaxSize()
            .padding(4.dp)) {
            val stroke = Stroke(width = 4.dp.toPx(), cap = StrokeCap.Round)
            drawArc(color = trackColor, startAngle = 0f, sweepAngle = 360f, useCenter = false, style = stroke)
            val sweep = if (row.state == "complete") 360f else 360f * percent
            if (sweep > 0f) {
                drawArc(color = arcColor, startAngle = -90f, sweepAngle = sweep, useCenter = false, style = stroke)
            }
        }
        Text(label, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurface)
    }
}

/** Two staggered arrows drifting through the ring: down = receiving, up = sending. */
/** Arrow dropping into a tray (receiving) or rising out of one (sending). */
@Composable
private fun FlowArrows(sending: Boolean, color: Color) {
    val t by rememberInfiniteTransition(label = "flow").animateFloat(
        initialValue = 0f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1400, easing = LinearEasing)),
        label = "flowProgress",
    )
    Canvas(modifier = Modifier.size(26.dp)) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val head = w * 0.2f
        val stroke = Stroke(width = 2.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round)
        // Receiving draws top-down; sending is the same drawing mirrored.
        fun y(v: Float) = if (sending) h - v else v

        // Pipe: a thick horizontal bar the arrow lands on.
        drawLine(
            color = color.copy(alpha = 0.4f),
            start = Offset(w * 0.28f, y(h * 0.82f)),
            end = Offset(w * 0.72f, y(h * 0.82f)),
            strokeWidth = 4.dp.toPx(),
            cap = StrokeCap.Round,
        )

        // Arrow: eases toward the tray, fades in at the start and out at the end.
        val p = FastOutSlowInEasing.transform((t / 0.8f).coerceIn(0f, 1f))
        val tip = h * 0.36f + (h * 0.32f) * p
        val a = when {
            t < 0.15f -> t / 0.15f
            t > 0.8f -> (1f - t) / 0.2f
            else -> 1f
        }.coerceIn(0f, 1f) * 0.6f
        val arrow = Path().apply {
            moveTo(cx, y(tip - h * 0.34f))
            lineTo(cx, y(tip))
            moveTo(cx - head, y(tip - head))
            lineTo(cx, y(tip))
            lineTo(cx + head, y(tip - head))
        }
        drawPath(arrow, color.copy(alpha = a), style = stroke)
    }
}

private fun transferSubtitle(row: FileTransferUi): String {
    val peer = row.deviceName.ifBlank { "another device" }
    val who = if (row.direction == "send") "To $peer" else "From $peer"
    val status = when (row.state) {
        "requesting" -> if (row.direction == "send") "Waiting" else "Starting"
        "receiving" -> "Receiving"
        "sending" -> "Sending"
        "saving" -> "Saving"
        "paused" -> "Paused"
        "interrupted" -> "Interrupted"
        "complete" -> "Done"
        "cancelled" -> "Cancelled"
        "expired" -> "Offer expired"
        "failed" -> row.message.ifBlank { "Failed" }
        else -> row.state
    }
    val size = if (row.total > 0L) "${transferSizeLabel(row.done)} of ${transferSizeLabel(row.total)}" else ""
    return listOf(who, status, size).filter { it.isNotBlank() }.joinToString("  •  ")
}

private fun transferSizeLabel(bytes: Long): String {
    if (bytes < 1024L) return "$bytes B"
    val units = arrayOf("KB", "MB", "GB", "TB")
    var value = bytes.toDouble()
    var unit = -1
    do {
        value /= 1024.0
        unit++
    } while (value >= 1024.0 && unit < units.lastIndex)
    return String.format(java.util.Locale.US, "%.1f %s", value, units[unit])
}
