// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder

@Serializable(with = CoordinatesSerializer::class)
data class Coordinates(
    val longitude: Double,
    val latitude: Double,
    val altitudeMeters: Double? = null
)

@Serializable
private class PointSurrogate(
    val type: String,
    val coordinates: List<Double>
) {
    init {
        require(type == "Point" && coordinates.size >= 2) {
            "Invalid GeoJSON Point: type=$type, coordinates=$coordinates"
        }
    }
}

object CoordinatesSerializer : KSerializer<Coordinates> {
    override val descriptor: SerialDescriptor = PointSurrogate.serializer().descriptor

    override fun serialize(encoder: Encoder, value: Coordinates) {
        val coords = if (value.altitudeMeters != null) {
            listOf(value.longitude, value.latitude, value.altitudeMeters)
        } else {
            listOf(value.longitude, value.latitude)
        }
        encoder.encodeSerializableValue(
            PointSurrogate.serializer(),
            PointSurrogate(type = "Point", coordinates = coords)
        )
    }

    override fun deserialize(decoder: Decoder): Coordinates {
        val surrogate = decoder.decodeSerializableValue(PointSurrogate.serializer())
        val alt = if (surrogate.coordinates.size >= 3) surrogate.coordinates[2] else null
        return Coordinates(
            longitude = surrogate.coordinates[0],
            latitude = surrogate.coordinates[1],
            altitudeMeters = alt
        )
    }
}

@Serializable
data class BoundingBox(
    @SerialName("min_lon")
    val minLongitude: Double,
    @SerialName("min_lat")
    val minLatitude: Double,
    @SerialName("max_lon")
    val maxLongitude: Double,
    @SerialName("max_lat")
    val maxLatitude: Double
) {
    fun contains(coordinates: Coordinates): Boolean {
        return coordinates.longitude in minLongitude..maxLongitude &&
                coordinates.latitude in minLatitude..maxLatitude
    }
}
