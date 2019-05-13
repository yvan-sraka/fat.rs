mod fat16;
use fat16::*;

fn browse_dir(pfx: String, fs: &mut FileSystem, dir: Directory) {
    let entries = fs.read_directory(dir).unwrap();

    for i in entries.iter() {
        let name =
            if i.extension() == "" { i.name().to_string() }
            else { format!("{}.{}", i.name(), i.extension()) };
        let pfx = format!("{}/{}", pfx, name);
        println!("entry: {}", pfx);
        match i.entry_type() {
            EntryType::Dir(dir) => {
                if !name.starts_with(".") {
                    browse_dir(pfx, fs, dir);
                }
            }
            EntryType::File(f) => ()
        }
    }
}

fn main() {
    let path = std::path::Path::new("test.img");
    let mut fs = FileSystem::new(path).unwrap();
    println!("FAT volume label {}, number of sectors {:x}, size {:x}",
    fs.volume_name(), fs.sectors_count(), fs.volume_size());

    let root = fs.root_directory();
    browse_dir("".to_string(), &mut fs, root);
}
