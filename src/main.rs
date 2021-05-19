mod fat32;
use fat32::*;

// recursively browse `dir` in `fs` and displays every element found
// pfx is used to display the whole path of every element
fn browse_dir(pfx: String, fs: &mut FAT32, dir: Directory) {
    let entries = fs.read_directory(dir).unwrap();

    for entry in entries.iter() {
        let name = entry.full_name();
        let path = format!("{}/{}", pfx, name);
        println!("{}", path);

        match entry.entry_type() {
            EntryType::Dir(dir) => {
                // ignore . and .. to avoid
                // infinite recursion
                if !name.starts_with('.') {
                    // recursively descend into
                    // sub directories
                    browse_dir(path, fs, dir);
                }
            }

            EntryType::File(_f) => (),
        }
    }
}

fn main() {
    let path = std::path::Path::new("imgs/fat32.img");
    let mut fs = FAT32::new(path).unwrap();
    println!(
        "FAT volume label {}, number of sectors {:x}, size {:x}",
        fs.volume_name(),
        fs.sector_count(),
        fs.volume_size()
    );

    let root = fs.root_directory();
    browse_dir("".to_string(), &mut fs, root);
}
