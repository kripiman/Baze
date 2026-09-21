// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.datetime.Instant
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
enum class HazardType {
    @SerialName("warning")
    WARNING,

    @SerialName("blocking")
    BLOCKING
}

@Serializable
enum class HazardStatus {
    @SerialName("unconfirmed")
    UNCONFIRMED,

    @SerialName("confirmed")
    CONFIRMED,

    @SerialName("resolved")
    RESOLVED
}

@Serializable
data class Hazard(
    val id: String,
    val category: String,
    val type: HazardType,
    val status: HazardStatus,
    val description: String? = null,
    val upvotes: Int,
    val downvotes: Int,
    val location: Coordinates,
    val createdAt: Instant,
    val expiresAt: Instant
) {
    val isConfirmedBlocking: Boolean
        get() = type == HazardType.BLOCKING && status == HazardStatus.CONFIRMED
}
