# AI Studio melalui RoboDhan

Jalankan RoboDhan hasil build integrasi, lalu PhotoCraft AI. API lokal RoboDhan menggunakan http://127.0.0.1:1234/v1. Tombol AI di sidebar menampilkan/menyembunyikan panel tanpa menghapus prompt atau hasil yang sedang diproses.

PhotoCraft mengirim prompt gambar ke /images/generations. Hasil PNG 512×512 atau 768×768 dibuka sebagai dokumen baru yang belum disimpan. Hubungkan dan Sempurnakan Prompt memakai model chat aktif di RoboDhan melalui /models dan /chat/completions. Gambar yang terbuka tidak dikirim untuk penyempurnaan prompt.

Editor berkomunikasi langsung melalui HTTP lokal; tidak menjalankan Python. RoboDhan memakai kembali runtime MFLUX yang sudah terpasang di PhotoCraft-AI-Backend. Jangan hapus atau pindahkan folder runtime karena lingkungan Python memakai jalur absolut. Tidak ada runtime kedua atau unduhan model baru.

Model gambar: mflux-community/flux2-klein-4b-mflux-q4, revisi 794cd159538149ad9830848508c31f0ea7088e58. Model lengkap berada di ~/.cache/huggingface/hub. Jalur model lama menjadi symlink agar tetap kompatibel. Model chat dan model gambar berbeda; FLUX digunakan oleh API gambar, bukan chat.

Abaikan hasil membuang respons ketika selesai, bukan menghentikan proses di RoboDhan. Tutup/hide panel tidak membatalkan proses. Satu proses gambar berjalan pada satu waktu; batas waktu 10 menit. Model gambar dibaca secara offline.

Build macOS ini ditandatangani lokal (ad-hoc), belum dinotarisasi Apple. RoboDhan integrasi dibangun tanpa App Sandbox agar dapat menjalankan runtime lokal dan membaca cache bersama. Konfigurasi Resources/local-ai.json menentukan runtime, mengaktifkan API lokal secara default, dan memilih Gemma sebagai model chat awal jika belum ada pilihan tersimpan.
