// Standalone Windows ARM64 NPU Worker for PhotoCraft
// Connects to Windows 11 Copilot+ PC native NPU via Microsoft.Windows.AI.Imaging
//
// Build: dotnet publish -c Release -r win-arm64 --self-contained
using System;
using System.IO;
using System.Threading.Tasks;
using Windows.Graphics.Imaging;
using Microsoft.Windows.AI;
using Microsoft.Windows.AI.Imaging;

class Program
{
    static async Task<int> Main(string[] args)
    {
        // 1. Verify NPU ImageForegroundExtractor ready state
        var readyState = ImageForegroundExtractor.GetReadyState();
        if (readyState == AIFeatureReadyState.NotReady)
        {
            var op = await ImageForegroundExtractor.EnsureReadyAsync();
            if (op.Status != AIFeatureReadyResultState.Success)
            {
                Console.Error.WriteLine("NPU ImageForegroundExtractor not ready: " + op.ExtendedError);
                return 1;
            }
        }
        else if (readyState != AIFeatureReadyState.Ready)
        {
            Console.Error.WriteLine("NPU ImageForegroundExtractor disabled or unsupported on this device.");
            return 2;
        }

        using var extractor = await ImageForegroundExtractor.CreateAsync();

        // 2. Read dimensions from binary stream (stdin)
        using var stdin = Console.OpenStandardInput();
        using var reader = new BinaryReader(stdin);

        int width = reader.ReadInt32();
        int height = reader.ReadInt32();
        byte[] bgraBytes = reader.ReadBytes(width * height * 4);

        // 3. Create SoftwareBitmap and execute on NPU
        var bitmap = SoftwareBitmap.CreateCopyFromBuffer(
            bgraBytes.AsBuffer(),
            BitmapPixelFormat.Bgra8,
            width,
            height,
            BitmapAlphaMode.Premultiplied
        );

        using var maskBitmap = extractor.GetMaskFromSoftwareBitmap(bitmap);

        // 4. Extract Gray8 mask bytes and stream back to PhotoCraft
        byte[] maskBytes = new byte[width * height];
        maskBitmap.CopyToBuffer(maskBytes.AsBuffer());

        using var stdout = Console.OpenStandardOutput();
        using var writer = new BinaryWriter(stdout);
        writer.Write(width);
        writer.Write(height);
        writer.Write(maskBytes);
        writer.Flush();

        return 0;
    }
}