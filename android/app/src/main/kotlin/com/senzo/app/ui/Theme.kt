package com.senzo.app.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/** The dark palette of the desktop application, in Compose form. */
val Background = Color(0xFF08090D)
val Surface = Color(0xFF12141B)
val SurfaceHigh = Color(0xFF1A1D26)
val Accent = Color(0xFF7C5CFF)
val TextPrimary = Color(0xFFF2F3F7)
val TextMuted = Color(0xFF9AA0AE)

private val SenzoColors = darkColorScheme(
    primary = Accent,
    onPrimary = Color.White,
    primaryContainer = Accent.copy(alpha = 0.18f),
    onPrimaryContainer = TextPrimary,
    background = Background,
    onBackground = TextPrimary,
    surface = Surface,
    onSurface = TextPrimary,
    surfaceVariant = SurfaceHigh,
    onSurfaceVariant = TextMuted,
    outline = Color(0xFF2A2E3A),
    error = Color(0xFFFF6B6B),
)

private val SenzoTypography = Typography(
    headlineMedium = TextStyle(fontSize = 26.sp, fontWeight = FontWeight.SemiBold),
    titleLarge = TextStyle(fontSize = 20.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontSize = 16.sp, fontWeight = FontWeight.Medium),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp),
    labelLarge = TextStyle(fontSize = 14.sp, fontWeight = FontWeight.Medium),
)

@Composable
fun SenzoTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = SenzoColors,
        typography = SenzoTypography,
        content = content,
    )
}
