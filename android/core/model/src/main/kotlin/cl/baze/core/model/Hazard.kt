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
enum class HazardCategory {
    @SerialName("glass")
    GLASS,

    @SerialName("pothole")
    POTHOLE,

    @SerialName("debris")
    DEBRIS,

    @SerialName("road_closed")
    ROAD_CLOSED,

    @SerialName("construction")
    CONSTRUCTION,

    @SerialName("flood")
    FLOOD;

    val hazardType: HazardType
        get() = when (this) {
            GLASS, POTHOLE, DEBRIS -> HazardType.WARNING
            ROAD_CLOSED, CONSTRUCTION, FLOOD -> HazardType.BLOCKING
        }
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
    val category: HazardCategory,
    @SerialName("hazard_type")
    val hazardType: HazardType = category.hazardType,
    val status: HazardStatus,
    val description: String? = null,
    val upvotes: Int,
    val downvotes: Int,
    val location: Coordinates,
    @SerialName("created_at")
    val createdAt: Instant,
    @SerialName("expires_at")
    val expiresAt: Instant
) {
    val isConfirmedBlocking: Boolean
        get() = hazardType == HazardType.BLOCKING && status == HazardStatus.CONFIRMED
}
