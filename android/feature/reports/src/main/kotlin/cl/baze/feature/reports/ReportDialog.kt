// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.feature.reports

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import cl.baze.core.model.Coordinates
import cl.baze.core.model.HazardType

/**
 * Diálogo de reporte de incidencias en ruta.
 * Permite seleccionar entre peligro menor ('warning') y bloqueo transitable ('blocking').
 */
@Composable
fun ReportHazardDialog(
    location: Coordinates,
    onDismiss: () -> Unit,
    onSubmit: (category: String, type: HazardType, description: String?) -> Unit
) {
    var category by remember { mutableStateOf("pothole") }
    var hazardType by remember { mutableStateOf(HazardType.WARNING) }
    var description by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Reportar peligro en la vía") },
        text = {
            Column(modifier = Modifier.fillMaxWidth().padding(8.dp)) {
                Text("Tipo de impacto:")
                Row(verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(
                        selected = hazardType == HazardType.WARNING,
                        onClick = { hazardType = HazardType.WARNING }
                    )
                    Text("Advertencia (vidrio, bache, gravilla)")
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    RadioButton(
                        selected = hazardType == HazardType.BLOCKING,
                        onClick = { hazardType = HazardType.BLOCKING }
                    )
                    Text("Bloqueo vial (corte total, inundación)")
                }

                Spacer(modifier = Modifier.height(8.dp))
                OutlinedTextField(
                    value = description,
                    onValueChange = { description = it },
                    label = { Text("Descripción opcional") },
                    modifier = Modifier.fillMaxWidth()
                )
            }
        },
        confirmButton = {
            Button(onClick = { onSubmit(category, hazardType, description.ifBlank { null }) }) {
                Text("Enviar reporte")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancelar")
            }
        }
    )
}
