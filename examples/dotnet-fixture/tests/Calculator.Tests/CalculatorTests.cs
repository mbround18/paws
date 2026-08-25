using Example;

namespace Example.Tests;

public class CalculatorTests
{
    [Fact]
    public void Adds()
    {
        Assert.Equal(5, Calculator.Add(2, 3));
    }

    [Fact]
    public void Multiplies()
    {
        Assert.Equal(6, Calculator.Multiply(2, 3));
    }
}
