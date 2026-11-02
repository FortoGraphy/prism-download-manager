//! Texts the shell shows itself (tray menu, tray tooltip, notifications), in the language the
//! UI uses. The main window reports its language (`set_ui_language`), which already resolves
//! "same as the system" the way the rest of the UI does.

use std::sync::RwLock;

static LANGUAGE: RwLock<String> = RwLock::new(String::new());

/// Remembers the UI language; returns whether it changed.
pub fn set_language(code: &str) -> bool {
    let mut lang = LANGUAGE.write().unwrap_or_else(|e| e.into_inner());
    if *lang == code {
        return false;
    }
    *lang = code.to_string();
    true
}

/// `key` (the English text) in the UI language, with `{name}` placeholders filled in.
pub fn tr(key: &str, vars: &[(&str, &str)]) -> String {
    let lang = LANGUAGE.read().unwrap_or_else(|e| e.into_inner()).clone();
    let mut text = lookup(&lang, key).unwrap_or(key).to_string();
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

fn lookup(lang: &str, key: &str) -> Option<&'static str> {
    let row = TEXTS.iter().find(|row| row[0] == key)?;
    let column = match lang {
        "cs" => 1,
        "de" => 2,
        "es" => 3,
        "fr" => 4,
        "it" => 5,
        "id" => 6,
        "ar" => 7,
        "tr" => 8,
        "zh" => 9,
        _ => return None,
    };
    Some(row[column])
}

/// English, Czech, German, Spanish, French, Italian, Indonesian, Arabic, Turkish, Simplified Chinese.
const TEXTS: &[[&str; 10]] = &[
    [
        "Open Prism",
        "Otevřít Prism",
        "Prism öffnen",
        "Abrir Prism",
        "Ouvrir Prism",
        "Apri Prism",
        "Buka Prism",
        "فتح Prism",
        "Prism'i aç",
        "打开 Prism",
    ],
    [
        "Add URL…",
        "Přidat adresu…",
        "URL hinzufügen…",
        "Añadir URL…",
        "Ajouter une URL…",
        "Aggiungi URL…",
        "Tambah URL…",
        "إضافة رابط…",
        "URL ekle…",
        "添加 URL…",
    ],
    [
        "Pause all",
        "Pozastavit vše",
        "Alle pausieren",
        "Pausar todo",
        "Tout suspendre",
        "Metti tutto in pausa",
        "Jeda semua",
        "إيقاف الكل مؤقتًا",
        "Tümünü duraklat",
        "全部暂停",
    ],
    [
        "Resume all",
        "Obnovit vše",
        "Alle fortsetzen",
        "Reanudar todo",
        "Tout reprendre",
        "Riprendi tutto",
        "Lanjutkan semua",
        "استئناف الكل",
        "Tümünü sürdür",
        "全部继续",
    ],
    [
        "Speed limiter",
        "Omezení rychlosti",
        "Geschwindigkeitslimit",
        "Limitador de velocidad",
        "Limiteur de vitesse",
        "Limitatore di velocità",
        "Pembatas kecepatan",
        "محدد السرعة",
        "Hız sınırlayıcı",
        "限速",
    ],
    [
        "Mini window",
        "Mini okno",
        "Mini-Fenster",
        "Mini ventana",
        "Mini-fenêtre",
        "Mini finestra",
        "Jendela mini",
        "نافذة مصغرة",
        "Mini pencere",
        "迷你窗口",
    ],
    [
        "Drop zone",
        "Odkládací zóna",
        "Ablagezone",
        "Zona para soltar",
        "Zone de dépôt",
        "Area di rilascio",
        "Zona lepas",
        "منطقة الإفلات",
        "Bırakma alanı",
        "拖放区",
    ],
    [
        "Hide tray icon",
        "Skrýt ikonu",
        "Symbol ausblenden",
        "Ocultar el icono",
        "Masquer l’icône",
        "Nascondi l’icona",
        "Sembunyikan ikon",
        "إخفاء الأيقونة",
        "Simgeyi gizle",
        "隐藏托盘图标",
    ],
    [
        "Quit Prism",
        "Ukončit Prism",
        "Prism beenden",
        "Salir de Prism",
        "Quitter Prism",
        "Esci da Prism",
        "Keluar dari Prism",
        "إنهاء Prism",
        "Prism'den çık",
        "退出 Prism",
    ],
    [
        "Prism — idle",
        "Prism — nic se nestahuje",
        "Prism — inaktiv",
        "Prism — inactivo",
        "Prism — inactif",
        "Prism — inattivo",
        "Prism — diam",
        "Prism — خامل",
        "Prism — boşta",
        "Prism — 空闲",
    ],
    [
        "Prism — {count} downloading · {speed}",
        "Prism — stahuje se: {count} · {speed}",
        "Prism — {count} aktiv · {speed}",
        "Prism — {count} descargando · {speed}",
        "Prism — {count} en cours · {speed}",
        "Prism — {count} in download · {speed}",
        "Prism — {count} mengunduh · {speed}",
        "Prism — {count} قيد التنزيل · {speed}",
        "Prism — {count} indiriliyor · {speed}",
        "Prism — {count} 个正在下载 · {speed}",
    ],
    [
        "Download added",
        "Stahování přidáno",
        "Download hinzugefügt",
        "Descarga añadida",
        "Téléchargement ajouté",
        "Download aggiunto",
        "Unduhan ditambahkan",
        "تمت إضافة التنزيل",
        "İndirme eklendi",
        "已添加下载",
    ],
    [
        "Download complete",
        "Stahování dokončeno",
        "Download abgeschlossen",
        "Descarga completada",
        "Téléchargement terminé",
        "Download completato",
        "Unduhan selesai",
        "اكتمل التنزيل",
        "İndirme tamamlandı",
        "下载完成",
    ],
    [
        "Download failed",
        "Stahování selhalo",
        "Download fehlgeschlagen",
        "La descarga ha fallado",
        "Échec du téléchargement",
        "Download non riuscito",
        "Unduhan gagal",
        "فشل التنزيل",
        "İndirme başarısız oldu",
        "下载失败",
    ],
    [
        "Queue finished",
        "Fronta dokončena",
        "Warteschlange abgeschlossen",
        "Cola terminada",
        "File d’attente terminée",
        "Coda completata",
        "Antrean selesai",
        "انتهت قائمة الانتظار",
        "Kuyruk tamamlandı",
        "队列已完成",
    ],
    [
        "{name} has no more downloads.",
        "Ve frontě {name} už nic nezbývá.",
        "{name} hat keine Downloads mehr.",
        "{name} no tiene más descargas.",
        "{name} n’a plus de téléchargements.",
        "{name} non ha altri download.",
        "{name} tidak punya unduhan lagi.",
        "لم يعد في {name} أي تنزيلات.",
        "{name} için başka indirme kalmadı.",
        "队列“{name}”中已没有待下载的项目。",
    ],
    [
        "Added to the list",
        "Přidáno do seznamu",
        "Zur Liste hinzugefügt",
        "Añadido a la lista",
        "Ajouté à la liste",
        "Aggiunto all’elenco",
        "Ditambahkan ke daftar",
        "أضيف إلى القائمة",
        "Listeye eklendi",
        "已添加到列表",
    ],
    [
        "Downloading in the browser",
        "Stahuje se v prohlížeči",
        "Download im Browser",
        "Descargando en el navegador",
        "Téléchargement dans le navigateur",
        "Download nel browser",
        "Mengunduh di browser",
        "يجري التنزيل في المتصفح",
        "Tarayıcıda indiriliyor",
        "正在浏览器中下载",
    ],
    [
        "{host} lets only a web browser download this file, so the browser is downloading it.",
        "{host} pouští stahování jen webovému prohlížeči, takže soubor stahuje prohlížeč.",
        "{host} lässt nur Webbrowser diese Datei herunterladen, daher lädt der Browser sie herunter.",
        "{host} solo deja descargar este archivo a un navegador web, así que lo descarga el navegador.",
        "{host} ne laisse qu’un navigateur web télécharger ce fichier, c’est donc le navigateur qui le télécharge.",
        "{host} permette di scaricare questo file solo a un browser web, quindi lo sta scaricando il browser.",
        "{host} hanya mengizinkan browser web mengunduh file ini, jadi browser yang mengunduhnya.",
        "يسمح {host} لمتصفح الويب فقط بتنزيل هذا الملف، لذا يتولى المتصفح تنزيله.",
        "{host} bu dosyanın yalnızca bir web tarayıcısıyla indirilmesine izin veriyor, bu yüzden dosyayı tarayıcı indiriyor.",
        "{host} 只允许网页浏览器下载此文件，因此改由浏览器下载。",
    ],
    [
        "Conversion finished",
        "Převod dokončen",
        "Umwandlung abgeschlossen",
        "Conversión terminada",
        "Conversion terminée",
        "Conversione completata",
        "Konversi selesai",
        "اكتمل التحويل",
        "Dönüştürme tamamlandı",
        "转换完成",
    ],
    [
        "Smaller file ready",
        "Menší soubor je hotový",
        "Kleinere Datei ist fertig",
        "Archivo más pequeño listo",
        "Fichier allégé prêt",
        "File più leggero pronto",
        "File yang lebih kecil sudah siap",
        "الملف الأصغر جاهز",
        "Küçültülmüş dosya hazır",
        "缩小后的文件已就绪",
    ],
    [
        "ZIP file ready",
        "Soubor ZIP je hotový",
        "ZIP-Datei ist fertig",
        "Archivo ZIP listo",
        "Fichier ZIP prêt",
        "File ZIP pronto",
        "File ZIP sudah siap",
        "ملف ZIP جاهز",
        "ZIP dosyası hazır",
        "ZIP 文件已就绪",
    ],
    [
        "Conversion failed",
        "Převod se nezdařil",
        "Umwandlung fehlgeschlagen",
        "La conversión falló",
        "Échec de la conversion",
        "Conversione non riuscita",
        "Konversi gagal",
        "فشل التحويل",
        "Dönüştürme başarısız oldu",
        "转换失败",
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_keeps_the_placeholders() {
        let names = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string()))
                .collect();
            v.sort();
            v
        };
        for row in TEXTS {
            for text in &row[1..] {
                assert_eq!(names(text), names(row[0]), "{text}");
                assert!(!text.trim().is_empty());
            }
        }
    }

    #[test]
    fn translates_and_falls_back_to_english() {
        set_language("cs");
        assert_eq!(tr("Pause all", &[]), "Pozastavit vše");
        assert_eq!(
            tr("{name} has no more downloads.", &[("name", "Noc")]),
            "Ve frontě Noc už nic nezbývá."
        );
        assert_eq!(tr("Something new", &[]), "Something new");
        set_language("tr");
        assert_eq!(tr("Open Prism", &[]), "Prism'i aç");
        set_language("de");
        assert_eq!(tr("Conversion finished", &[]), "Umwandlung abgeschlossen");
        set_language("zh");
        assert_eq!(tr("Pause all", &[]), "全部暂停");
        assert_eq!(
            tr("{name} has no more downloads.", &[("name", "夜间")]),
            "队列“夜间”中已没有待下载的项目。"
        );
        set_language("en");
        assert_eq!(tr("Pause all", &[]), "Pause all");
        set_language("ja");
        assert_eq!(tr("Pause all", &[]), "Pause all");
    }
}
